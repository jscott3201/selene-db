//! Executor binding-table tests.

mod exec_common;

use selene_core::Value;
use selene_gql::{Binding, BindingTable, BindingTableSchema};

#[test]
fn binding_table_tracks_schema_and_rows() {
    let schema = BindingTableSchema { columns: vec![] };
    let empty = BindingTable::empty(schema.clone());
    assert!(empty.is_empty());
    let table = BindingTable::new(
        schema,
        vec![Binding::new([Value::Int(1), Value::Bool(true)])],
    );

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(1), Value::Bool(true)]
    );
    assert_eq!(table.iter().count(), 1);
}
