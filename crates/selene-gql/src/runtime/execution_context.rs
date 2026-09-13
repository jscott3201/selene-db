//! Semantic execution records, contexts, and deterministic context stack.

use std::{collections::BTreeMap, sync::Arc};

use selene_core::{DbString, Value};

use super::{BindingTable, ExecutionOutcome};

/// Immutable named-value record used by one execution context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    fields: BTreeMap<DbString, Value>,
}

impl Record {
    /// Construct an empty record.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    /// Construct a record, rejecting duplicate names instead of overwriting.
    pub fn new(
        fields: impl IntoIterator<Item = (DbString, Value)>,
    ) -> Result<Self, ExecutionContextError> {
        let mut record = Self::empty();
        for (name, value) in fields {
            if record.fields.insert(name.clone(), value).is_some() {
                return Err(ExecutionContextError::DuplicateRecordField { name });
            }
        }
        Ok(record)
    }

    /// Borrow one exact-name field.
    #[must_use]
    pub fn get(&self, name: &DbString) -> Option<&Value> {
        self.fields.get(name)
    }

    /// Iterate fields in deterministic lexical order.
    pub fn iter(&self) -> impl Iterator<Item = (&DbString, &Value)> {
        self.fields.iter()
    }

    /// Return a new record amended with disjoint fields.
    pub fn amend(
        &self,
        fields: impl IntoIterator<Item = (DbString, Value)>,
    ) -> Result<Self, ExecutionContextError> {
        let amendment = Self::new(fields)?;
        let mut next = self.fields.clone();
        for (name, value) in amendment.fields {
            if next.insert(name.clone(), value).is_some() {
                return Err(ExecutionContextError::DuplicateRecordField { name });
            }
        }
        Ok(Self { fields: next })
    }
}

/// Mutable context shell whose record, table, and outcome values are immutable.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionContext {
    record: Arc<Record>,
    table: Arc<BindingTable>,
    outcome: ExecutionOutcome,
}

impl ExecutionContext {
    /// Construct the root context: empty record, unit table, omitted success.
    #[must_use]
    pub fn root() -> Self {
        Self {
            record: Arc::new(Record::empty()),
            table: Arc::new(BindingTable::unit()),
            outcome: ExecutionOutcome::successful_omitted(),
        }
    }

    /// Construct a context after checking record/table field disjointness.
    pub fn new(
        record: Arc<Record>,
        table: Arc<BindingTable>,
        outcome: ExecutionOutcome,
    ) -> Result<Self, ExecutionContextError> {
        ensure_disjoint(&record, &table)?;
        Ok(Self {
            record,
            table,
            outcome,
        })
    }

    fn child(&self) -> Self {
        Self {
            record: Arc::clone(&self.record),
            table: Arc::new(BindingTable::unit()),
            outcome: self.outcome.clone(),
        }
    }

    /// Borrow the immutable working record.
    #[must_use]
    pub fn record(&self) -> &Record {
        &self.record
    }

    /// Borrow the immutable working table.
    #[must_use]
    pub fn table(&self) -> &BindingTable {
        &self.table
    }

    /// Borrow the current structured outcome.
    #[must_use]
    pub const fn outcome(&self) -> &ExecutionOutcome {
        &self.outcome
    }

    /// Replace the working record after checking table-field overlap.
    pub fn replace_record(&mut self, record: Arc<Record>) -> Result<(), ExecutionContextError> {
        ensure_disjoint(&record, &self.table)?;
        self.record = record;
        Ok(())
    }

    /// Amend the working record without mutating a shared parent record.
    pub fn amend_record(
        &mut self,
        fields: impl IntoIterator<Item = (DbString, Value)>,
    ) -> Result<(), ExecutionContextError> {
        self.replace_record(Arc::new(self.record.amend(fields)?))
    }

    /// Replace the working table after checking record-field overlap.
    pub fn replace_table(&mut self, table: Arc<BindingTable>) -> Result<(), ExecutionContextError> {
        ensure_disjoint(&self.record, &table)?;
        self.table = table;
        Ok(())
    }

    /// Replace the current outcome.
    pub fn replace_outcome(&mut self, outcome: ExecutionOutcome) {
        self.outcome = outcome;
    }
}

/// Deterministic strict-LIFO stack with one permanent root context.
#[derive(Debug)]
pub struct ExecutionStack {
    contexts: Vec<ExecutionContext>,
}

impl ExecutionStack {
    /// Construct a stack containing exactly its root context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            contexts: vec![ExecutionContext::root()],
        }
    }

    /// Return the active stack depth, including the root.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.contexts.len()
    }

    /// Borrow the current top context.
    #[must_use]
    pub fn current(&self) -> &ExecutionContext {
        self.contexts
            .last()
            .expect("execution stack always retains its root")
    }

    /// Mutably borrow the active context shell for explicit replacement/amendment.
    pub fn current_mut(&mut self) -> &mut ExecutionContext {
        self.contexts
            .last_mut()
            .expect("execution stack always retains its root")
    }

    /// Push a child and return an RAII frame that pops on every exit path.
    pub fn push_child(&mut self) -> ExecutionFrame<'_> {
        let depth = self.contexts.len();
        let child = self.current().child();
        self.contexts.push(child);
        ExecutionFrame { stack: self, depth }
    }

    /// Push a child with an operation-supplied table override.
    pub fn push_child_with_table(
        &mut self,
        table: Arc<BindingTable>,
    ) -> Result<ExecutionFrame<'_>, ExecutionContextError> {
        let depth = self.contexts.len();
        let mut child = self.current().child();
        child.replace_table(table)?;
        self.contexts.push(child);
        Ok(ExecutionFrame { stack: self, depth })
    }
}

impl Default for ExecutionStack {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrowed stack frame that guarantees strict-LIFO cleanup on drop.
pub struct ExecutionFrame<'a> {
    stack: &'a mut ExecutionStack,
    depth: usize,
}

impl ExecutionFrame<'_> {
    /// Borrow the current child context.
    #[must_use]
    pub fn context(&self) -> &ExecutionContext {
        self.stack.current()
    }

    /// Mutably borrow the child shell to replace record, table, or outcome.
    pub fn context_mut(&mut self) -> &mut ExecutionContext {
        self.stack
            .contexts
            .last_mut()
            .expect("an execution frame always owns one child")
    }

    /// Push a nested child whose drop restores this frame.
    pub fn push_child(&mut self) -> ExecutionFrame<'_> {
        self.stack.push_child()
    }
}

impl Drop for ExecutionFrame<'_> {
    fn drop(&mut self) {
        self.stack.contexts.truncate(self.depth);
    }
}

/// Construction/amendment failure for semantic execution state.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutionContextError {
    /// A record constructor or amendment repeated one exact field name.
    #[error("duplicate execution-record field {name}")]
    DuplicateRecordField {
        /// Repeated field name.
        name: DbString,
    },
    /// A binding-table descriptor repeated one exact field name.
    #[error("duplicate working-table field {name}")]
    DuplicateTableField {
        /// Repeated field name.
        name: DbString,
    },
    /// The working record and working table contain the same exact field name.
    #[error("working record and table overlap at field {name}")]
    OverlappingField {
        /// Overlapping field name.
        name: DbString,
    },
}

fn ensure_disjoint(record: &Record, table: &BindingTable) -> Result<(), ExecutionContextError> {
    let mut table_fields = std::collections::BTreeSet::new();
    for column in &table.schema().columns {
        let Some(name) = &column.name else {
            continue;
        };
        if !table_fields.insert(name.clone()) {
            return Err(ExecutionContextError::DuplicateTableField { name: name.clone() });
        }
        if record.fields.contains_key(name) {
            return Err(ExecutionContextError::OverlappingField { name: name.clone() });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use crate::{AnalyzedType, BindingTableColumn, BindingTableSchema};
    use selene_core::db_string;

    use super::*;

    fn name(value: &str) -> DbString {
        db_string(value).unwrap()
    }

    fn named_table(value: &str) -> Arc<BindingTable> {
        Arc::new(BindingTable::new(
            BindingTableSchema {
                columns: vec![BindingTableColumn {
                    name: Some(name(value)),
                    hidden: None,
                    ty: AnalyzedType::Dynamic,
                }],
            },
            Vec::new(),
        ))
    }

    #[test]
    fn root_and_child_follow_copy_unit_and_isolation_contract() {
        let mut stack = ExecutionStack::new();
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.current().table().row_count(), 1);
        assert!(stack.current().record().iter().next().is_none());
        assert!(matches!(
            stack.current().outcome(),
            ExecutionOutcome::OmittedResult { .. }
        ));

        stack
            .current_mut()
            .amend_record([(name("parent"), Value::Int(7))])
            .unwrap();
        let inherited_outcome = ExecutionOutcome::failed(crate::DiagnosticBundle::new(
            crate::GqlStatusObject::new(crate::GqlStatus::DATA_EXCEPTION, "parent outcome"),
            Vec::new(),
        ));
        stack
            .current_mut()
            .replace_outcome(inherited_outcome.clone());

        {
            let mut child = stack.push_child();
            assert_eq!(
                child.context().record().get(&name("parent")),
                Some(&Value::Int(7))
            );
            assert_eq!(child.context().outcome(), &inherited_outcome);
            child
                .context_mut()
                .amend_record([(name("child"), Value::Int(1))])
                .unwrap();
            assert_eq!(child.context().table().row_count(), 1);
            assert_eq!(
                child.context().record().get(&name("child")),
                Some(&Value::Int(1))
            );
        }

        assert_eq!(stack.depth(), 1);
        assert_eq!(
            stack.current().record().get(&name("parent")),
            Some(&Value::Int(7))
        );
        assert!(stack.current().record().get(&name("child")).is_none());
    }

    #[test]
    fn construction_and_amendment_reject_every_silent_overwrite() {
        let duplicate =
            Record::new([(name("x"), Value::Int(1)), (name("x"), Value::Int(2))]).unwrap_err();
        assert!(matches!(
            duplicate,
            ExecutionContextError::DuplicateRecordField { .. }
        ));

        let record = Arc::new(Record::new([(name("x"), Value::Int(1))]).unwrap());
        let overlap = ExecutionContext::new(
            record,
            named_table("x"),
            ExecutionOutcome::successful_omitted(),
        )
        .unwrap_err();
        assert!(matches!(
            overlap,
            ExecutionContextError::OverlappingField { .. }
        ));

        let duplicate_name = name("duplicate");
        let duplicate_table = Arc::new(BindingTable::new(
            BindingTableSchema {
                columns: vec![
                    BindingTableColumn {
                        name: Some(duplicate_name.clone()),
                        hidden: None,
                        ty: AnalyzedType::Dynamic,
                    },
                    BindingTableColumn {
                        name: Some(duplicate_name),
                        hidden: None,
                        ty: AnalyzedType::Dynamic,
                    },
                ],
            },
            Vec::new(),
        ));
        assert!(matches!(
            ExecutionContext::new(
                Arc::new(Record::empty()),
                duplicate_table,
                ExecutionOutcome::successful_omitted(),
            ),
            Err(ExecutionContextError::DuplicateTableField { .. })
        ));

        let mut context = ExecutionContext::root();
        context.amend_record([(name("x"), Value::Int(1))]).unwrap();
        assert!(matches!(
            context.amend_record([(name("x"), Value::Int(2))]),
            Err(ExecutionContextError::DuplicateRecordField { .. })
        ));
        assert!(matches!(
            context.replace_table(named_table("x")),
            Err(ExecutionContextError::OverlappingField { .. })
        ));

        let mut stack = ExecutionStack::new();
        stack
            .current_mut()
            .amend_record([(name("x"), Value::Int(1))])
            .unwrap();
        assert!(matches!(
            stack.push_child_with_table(named_table("x")),
            Err(ExecutionContextError::OverlappingField { .. })
        ));
        assert_eq!(stack.depth(), 1, "a rejected push must not leak a child");
    }

    #[test]
    fn frames_clean_up_after_success_error_and_panic() {
        let mut stack = ExecutionStack::new();
        {
            let _frame = stack.push_child();
        }
        assert_eq!(stack.depth(), 1);

        let returned: Result<(), &'static str> = {
            let _frame = stack.push_child();
            Err("returned failure")
        };
        assert_eq!(returned, Err("returned failure"));
        assert_eq!(stack.depth(), 1);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let mut outer = stack.push_child();
            let _inner = outer.push_child();
            panic!("injected unwind");
        }));
        assert!(panic.is_err());
        assert_eq!(stack.depth(), 1);

        for _ in 0..1_000 {
            let _frame = stack.push_child();
        }
        assert_eq!(stack.depth(), 1, "hostile repetition must not leak frames");
    }
}
