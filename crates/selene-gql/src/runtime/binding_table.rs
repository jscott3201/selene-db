//! Row-at-a-time binding table representation.

use smallvec::SmallVec;

use selene_core::{IStr, Value};

use crate::plan::BindingTableSchema;

/// One executor binding-table row.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    values: SmallVec<[Value; 8]>,
}

impl Binding {
    /// Construct a row from ordered values.
    #[must_use]
    pub fn new(values: impl IntoIterator<Item = Value>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    /// Construct an empty row.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            values: SmallVec::new(),
        }
    }

    /// Borrow the row's values.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Borrow one value by column index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }
}

/// Executor binding table with schema and row storage.
#[derive(Clone, Debug, PartialEq)]
pub struct BindingTable {
    schema: BindingTableSchema,
    rows: Vec<Binding>,
}

impl BindingTable {
    /// Construct an empty table with the supplied schema.
    #[must_use]
    pub fn empty(schema: BindingTableSchema) -> Self {
        Self {
            schema,
            rows: Vec::new(),
        }
    }

    /// Construct a table from a schema and row vector.
    #[must_use]
    pub fn new(schema: BindingTableSchema, rows: Vec<Binding>) -> Self {
        Self { schema, rows }
    }

    /// Borrow the table schema.
    #[must_use]
    pub const fn schema(&self) -> &BindingTableSchema {
        &self.schema
    }

    /// Borrow all rows.
    #[must_use]
    pub fn rows(&self) -> &[Binding] {
        &self.rows
    }

    /// Iterate rows in storage order.
    pub fn iter(&self) -> impl Iterator<Item = &Binding> {
        self.rows.iter()
    }

    /// Return the number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Return true when the table has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Append one row.
    pub fn push_row(&mut self, row: Binding) {
        self.rows.push(row);
    }

    /// Return the index of the first named column matching `name`.
    #[must_use]
    pub fn column_index(&self, name: IStr) -> Option<usize> {
        self.schema
            .columns
            .iter()
            .position(|column| column.name == Some(name))
    }
}
