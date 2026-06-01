//! BRIEF-153: end-to-end test that binding a `Value::ExternalString` to a
//! `$p :: STRING` GQL parameter and inserting into a `STRING NOT NULL
//! INDEXED` column reaches the new property-index admission carve-out.
//!
//! Lives in its own file rather than alongside the BRIEF-115 parameter
//! tests because adding it to `parameters.rs` would push that file past
//! the 700-LOC per-file cap.

use std::sync::Arc;

use selene_core::{GraphId, IStr, Value, intern, lookup};
use selene_gql::{EmptyProcedureRegistry, ExecutorError, Session, StatementOutput};
use selene_graph::{SharedGraph, TypedIndexKind};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn execute(session: &mut Session<'_>, source: &str) -> Result<StatementOutput, ExecutorError> {
    session.execute_source(source, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

#[test]
fn external_string_parameter_lands_in_indexed_string_column() {
    // Bind a `Value::ExternalString` to a `$p :: STRING` parameter,
    // INSERT against a `STRING NOT NULL INDEXED` schema, and assert the
    // row is reachable through the property index from both the
    // `Value::String` (admitted) and `Value::ExternalString` (raw) probe
    // variants. Closes the silent-skip footgun for downstream consumers
    // that bind `Value::ExternalString` through GQL.
    let graph = SharedGraph::new(GraphId::new(4900));
    let function_label = istr("Brief153Function");
    let qualified_name = istr("qualified_name");
    graph
        .create_property_index(
            function_label.clone(),
            qualified_name.clone(),
            TypedIndexKind::String,
        )
        .expect("registers");

    let probe = "brief153::ext::unique_qualified_name";
    assert!(
        lookup(probe).is_none(),
        "fresh probe content must not already be in the IStr pool"
    );

    let mut session = Session::new(&graph);
    session.bind_parameter(istr("p"), Value::ExternalString(Arc::<str>::from(probe)));
    let table = rows(
        execute(
            &mut session,
            "INSERT (n:Brief153Function {qualified_name: $p :: STRING}) \
             RETURN n.qualified_name AS qn",
        )
        .expect("typed-parameter insert executes"),
    );
    assert_eq!(table.row_count(), 1);

    // After commit the IStr is in the pool (admission landed at the
    // index-commit boundary) and the index returns the new row from
    // either probe variant.
    let admitted = lookup(probe).expect("commit admitted the IStr");
    let snapshot = graph.read();
    let rows_via_string = snapshot
        .nodes_with_property_eq(&function_label, &qualified_name, &Value::String(admitted))
        .expect("kind matches");
    let rows_via_external = snapshot
        .nodes_with_property_eq(
            &function_label,
            &qualified_name,
            &Value::ExternalString(Arc::<str>::from(probe)),
        )
        .expect("kind matches");
    assert!(!rows_via_string.is_empty());
    assert!(!rows_via_external.is_empty());
    assert_eq!(
        rows_via_string.iter().collect::<Vec<_>>(),
        rows_via_external.iter().collect::<Vec<_>>(),
    );
}
