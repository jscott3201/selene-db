//! End-to-end coverage for native `selene.*` JSON built-ins.

use selene_core::{DbString, GraphId, JsonValue, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{
    AnalysisError, BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureRegistry,
    Session, StatementOutput, TypeMismatchContext,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn json(value: serde_json::Value) -> Value {
    Value::Json(JsonValue::new(value).expect("JSON is valid"))
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn seed_json_docs(graph: &SharedGraph) {
    let doc = db_string("JsonDoc");
    let other = db_string("OtherDoc");
    let payload = db_string("payload");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    for (label, value) in [
        (
            doc.clone(),
            json(serde_json::json!({"memory": {"kind": "episodic", "score": 7}})),
        ),
        (
            doc.clone(),
            json(serde_json::json!({"memory": {"kind": "semantic"}})),
        ),
        (doc.clone(), Value::String(db_string("not json"))),
        (
            doc,
            json(serde_json::json!(["agent", {"memory": {"kind": "episodic"}}])),
        ),
        (
            other,
            json(serde_json::json!({"memory": {"kind": "episodic"}})),
        ),
    ] {
        mutator
            .create_node(LabelSet::single(label), props(&payload, value))
            .expect("JSON node inserts");
    }
    txn.commit().expect("seed commits");
}

#[test]
fn json_contains_nodes_returns_json_property_candidates() {
    let graph = graph(514_101);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_contains_nodes(
             'JsonDoc',
             'payload',
             json('{"memory":{"kind":"episodic"}}'),
             10
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(1), NodeId::new(4)]
    );
}

#[test]
fn json_contains_nodes_respects_k() {
    let graph = graph(514_102);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_contains_nodes(
             'JsonDoc',
             'payload',
             json('{"memory":{"kind":"episodic"}}'),
             1
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(1)]);
}

#[test]
fn json_contains_nodes_rejects_non_json_candidate() {
    let graph = graph(514_103);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.json_contains_nodes('JsonDoc', 'payload', 'not json', 10)",
            &registry,
        )
        .expect_err("non-JSON candidate should fail");

    let ExecutorError::Analysis {
        source:
            AnalysisError::TypeMismatch {
                context: TypeMismatchContext::ProcedureArgument { position: 2, .. },
                ..
            },
    } = err
    else {
        panic!("expected analyzer JSON argument type mismatch, got {err:?}");
    };
}
