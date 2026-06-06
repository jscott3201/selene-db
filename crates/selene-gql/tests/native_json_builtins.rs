//! End-to-end coverage for native `selene.*` JSON built-ins.

use selene_core::{DbString, GraphId, JsonValue, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{
    AnalysisError, BindingTable, BuiltinProcedureRegistry, ExecutorError, GqlType, ProcedureError,
    ProcedureRegistry, Session, StatementOutput, TypeMismatchContext,
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

fn json_column(table: &BindingTable, name: &str) -> Vec<serde_json::Value> {
    let index = table
        .column_index(db_string(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Json(value)) => value.as_serde().clone(),
            other => panic!("expected JSON value in {name}, got {other:?}"),
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
fn json_path_exists_nodes_returns_json_property_candidates() {
    let graph = graph(514_104);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_exists_nodes(
             'JsonDoc',
             'payload',
             json_array('memory', 'score'),
             10
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(1)]);
}

#[test]
fn json_path_exists_nodes_supports_array_indexes_and_k() {
    let graph = graph(514_105);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_exists_nodes(
             'JsonDoc',
             'payload',
             json_array(1, 'memory', 'kind'),
             1
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(4)]);
}

#[test]
fn json_path_contains_nodes_returns_selected_subvalue_candidates() {
    let graph = graph(514_110);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_contains_nodes(
             'JsonDoc',
             'payload',
             json_array('memory'),
             json('{"kind":"episodic"}'),
             10
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(1)]);
}

#[test]
fn json_path_contains_nodes_supports_array_indexes_and_k() {
    let graph = graph(514_111);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_contains_nodes(
             'JsonDoc',
             'payload',
             json_array(1, 'memory'),
             json('{"kind":"episodic"}'),
             1
           ) YIELD node_id"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(4)]);
}

#[test]
fn json_contains_nodes_metadata_has_json_candidate() {
    let registry = BuiltinProcedureRegistry::new();
    let name = [db_string("selene"), db_string("json_contains_nodes")];
    let metadata = registry
        .lookup(&name)
        .expect("json_contains_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 4);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "candidate");
    assert_eq!(parameters[2].ty, GqlType::Json);
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
}

#[test]
fn json_path_exists_nodes_metadata_has_json_path() {
    let registry = BuiltinProcedureRegistry::new();
    let name = [db_string("selene"), db_string("json_path_exists_nodes")];
    let metadata = registry
        .lookup(&name)
        .expect("json_path_exists_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 4);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "path");
    assert_eq!(parameters[2].ty, GqlType::Json);
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
}

#[test]
fn json_path_contains_nodes_metadata_has_json_path_and_candidate() {
    let registry = BuiltinProcedureRegistry::new();
    let name = [db_string("selene"), db_string("json_path_contains_nodes")];
    let metadata = registry
        .lookup(&name)
        .expect("json_path_contains_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 5);
    assert_eq!(arity.maximum, 5);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "path");
    assert_eq!(parameters[2].ty, GqlType::Json);
    assert_eq!(parameters[3].name.as_str(), "candidate");
    assert_eq!(parameters[3].ty, GqlType::Json);
    assert_eq!(parameters[4].name.as_str(), "k");
    assert_eq!(parameters[4].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
}

#[test]
fn json_path_value_nodes_returns_selected_json_values() {
    let graph = graph(514_108);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_value_nodes(
             'JsonDoc',
             'payload',
             json_array('memory', 'kind'),
             10
           ) YIELD node_id, value"#,
        &registry,
    );

    assert_eq!(
        node_column(&table, "node_id"),
        vec![NodeId::new(1), NodeId::new(2)]
    );
    assert_eq!(
        json_column(&table, "value"),
        vec![serde_json::json!("episodic"), serde_json::json!("semantic")]
    );
}

#[test]
fn json_path_value_nodes_supports_array_indexes_and_k() {
    let graph = graph(514_109);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    seed_json_docs(&graph);

    let table = execute_rows(
        &mut session,
        r#"CALL selene.json_path_value_nodes(
             'JsonDoc',
             'payload',
             json_array(1, 'memory', 'kind'),
             1
           ) YIELD node_id, value"#,
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![NodeId::new(4)]);
    assert_eq!(
        json_column(&table, "value"),
        vec![serde_json::json!("episodic")]
    );
}

#[test]
fn json_path_value_nodes_metadata_has_json_path_and_value_output() {
    let registry = BuiltinProcedureRegistry::new();
    let name = [db_string("selene"), db_string("json_path_value_nodes")];
    let metadata = registry
        .lookup(&name)
        .expect("json_path_value_nodes resolves");

    let arity = metadata.signature.arity();
    assert_eq!(arity.minimum, 4);
    assert_eq!(arity.maximum, 4);
    let parameters = &metadata.signature.parameters;
    assert_eq!(parameters[0].name.as_str(), "label");
    assert_eq!(parameters[0].ty, GqlType::String);
    assert_eq!(parameters[1].name.as_str(), "property");
    assert_eq!(parameters[1].ty, GqlType::String);
    assert_eq!(parameters[2].name.as_str(), "path");
    assert_eq!(parameters[2].ty, GqlType::Json);
    assert_eq!(parameters[3].name.as_str(), "k");
    assert_eq!(parameters[3].ty, GqlType::Integer);

    let columns = &metadata.output_schema.columns;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].name.as_str(), "node_id");
    assert_eq!(columns[0].ty, GqlType::NodeRef);
    assert_eq!(columns[1].name.as_str(), "value");
    assert_eq!(columns[1].ty, GqlType::Json);
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

#[test]
fn json_path_contains_nodes_rejects_non_json_candidate() {
    let graph = graph(514_112);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            "CALL selene.json_path_contains_nodes('JsonDoc', 'payload', json_array('memory'), 'not json', 10)",
            &registry,
        )
        .expect_err("non-JSON candidate should fail");

    let ExecutorError::Analysis {
        source:
            AnalysisError::TypeMismatch {
                context: TypeMismatchContext::ProcedureArgument { position: 3, .. },
                ..
            },
    } = err
    else {
        panic!("expected analyzer JSON argument type mismatch, got {err:?}");
    };
}

#[test]
fn json_path_exists_nodes_rejects_bad_path_document() {
    let graph = graph(514_106);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);

    let err = session
        .execute_source(
            r#"CALL selene.json_path_exists_nodes('JsonDoc', 'payload', json_object('a', 1), 10)"#,
            &registry,
        )
        .expect_err("object path document should fail");

    let ExecutorError::Procedure {
        source: ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected procedure invalid argument, got {err:?}");
    };
    assert!(detail.contains("path must be a JSON array"));
}

#[test]
fn json_path_exists_nodes_rejects_too_many_selectors() {
    let graph = graph(514_107);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let selectors = std::iter::repeat_n("\"a\"", 65)
        .collect::<Vec<_>>()
        .join(", ");
    let json_path = format!("[{selectors}]");
    let source = format!(
        "CALL selene.json_path_exists_nodes('JsonDoc', 'payload', json('{json_path}'), 10)"
    );

    let err = session
        .execute_source(&source, &registry)
        .expect_err("over-limit path document should fail");

    let ExecutorError::Procedure {
        source: ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected procedure invalid argument, got {err:?}");
    };
    assert!(detail.contains("supports at most 64 selectors"));
}
