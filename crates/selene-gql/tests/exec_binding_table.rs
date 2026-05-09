//! Executor binding-table tests.

mod exec_common;

use selene_core::Value;
use selene_gql::{Binding, BindingTable, BindingTableSchema};

#[test]
fn binding_table_tracks_schema_and_rows() {
    let schema = BindingTableSchema { columns: vec![] };
    let mut table = BindingTable::empty(schema);

    assert!(table.is_empty());
    table.push_row(Binding::new([Value::Int(1), Value::Bool(true)]));

    assert_eq!(table.row_count(), 1);
    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(1), Value::Bool(true)]
    );
    assert_eq!(table.iter().count(), 1);
}
