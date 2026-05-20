//! EXPLAIN pipeline operator.

use std::sync::Arc;

use selene_core::{Value, intern_with_admission};

use crate::{
    AnalyzedType, BindingTableColumn, BindingTableSchema, ExecutionPlan, GqlType,
    runtime::{Binding, BindingTable, ExecutorError},
};

pub(super) fn execute(inner: &ExecutionPlan) -> Result<BindingTable, ExecutorError> {
    let dump = Arc::<str>::from(format!("{inner:#?}"));
    Ok(BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(intern_runtime("plan")?),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            }],
        },
        vec![Binding::new([Value::ExternalString(dump)])],
    ))
}

fn intern_runtime(value: &str) -> Result<selene_core::IStr, ExecutorError> {
    intern_with_admission(value)
        .map(|(value, _was_new)| value)
        .map_err(|_err| ExecutorError::ImplementationDefined {
            detail: "interner cap exhausted during EXPLAIN rendering",
        })
}
