//! Scan binding-slot resolution and row materialization.

use selene_core::Value;

use crate::{
    NodeOrEdgeScan, PatternPlan,
    runtime::{Binding, BindingTableSchema, ExecutorError},
};

use super::pattern;

#[derive(Clone, Copy)]
pub(super) struct ScanSlots {
    binding: pattern::ColumnSlot,
    hidden: pattern::ColumnSlot,
}

impl ScanSlots {
    pub(super) fn resolve(
        scan: &NodeOrEdgeScan,
        pattern: &PatternPlan,
        schema: &BindingTableSchema,
    ) -> Result<Self, ExecutorError> {
        Ok(Self {
            binding: pattern::ColumnSlot::binding(
                pattern,
                schema,
                scan.binding,
                "binding column missing from pattern schema",
            )?,
            hidden: pattern::ColumnSlot::hidden(
                schema,
                scan.hidden_binding,
                "hidden binding column missing from pattern schema",
            )?,
        })
    }

    pub(super) fn binding_index(self) -> Option<usize> {
        self.binding.index()
    }
}

pub(super) fn binding_for_scan(
    schema: &BindingTableSchema,
    seed: Option<&Binding>,
    entity: Value,
    slots: ScanSlots,
) -> Option<Binding> {
    let mut values = if let Some(row) = seed {
        let mut values = row.values().to_vec();
        values.resize(schema.columns.len(), Value::Null);
        values
    } else {
        vec![Value::Null; schema.columns.len()]
    };
    if !slots.binding.set(&mut values, entity.clone()) {
        return None;
    }
    if !slots.hidden.set(&mut values, entity) {
        return None;
    }
    Some(Binding::new(values))
}
