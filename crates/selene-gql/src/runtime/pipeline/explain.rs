//! EXPLAIN pipeline operator.

use selene_core::{Value, intern};

use crate::{
    AnalyzedType, BindingTableColumn, BindingTableSchema, ExecutionPlan, GqlType,
    runtime::{Binding, BindingTable, ExecutorError},
};

pub(super) fn execute(inner: &ExecutionPlan) -> Result<BindingTable, ExecutorError> {
    let dump = format!("{inner:#?}");
    Ok(BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(intern_runtime("plan")?),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::String),
            }],
        },
        vec![Binding::new([Value::String(intern_runtime(&dump)?)])],
    ))
}

fn intern_runtime(value: &str) -> Result<selene_core::IStr, ExecutorError> {
    intern(value).map_err(|_err| ExecutorError::ImplementationDefined {
        detail: "interner cap exhausted during EXPLAIN rendering",
    })
}
