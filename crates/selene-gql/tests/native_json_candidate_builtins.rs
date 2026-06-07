//! End-to-end coverage for candidate-scoped native JSON built-ins.

use selene_core::{DbString, GraphId, JsonValue, LabelSet, NodeId, PropertyMap, Value};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, GqlType, ProcedureError,
    ProcedureRegistry, Session, StatementOutput,
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

struct SeedIds {
    scoped_semantic: NodeId,
    scoped_array: NodeId,
}

fn seed_json_candidate_docs(graph: &SharedGraph) -> SeedIds {
    let doc = db_string("JsonDoc");
    let scope = db_string("JsonScope");
    let other = db_string("OtherDoc");
    let payload = db_string("payload");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();

    mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(
                &payload,
                json(serde_json::json!({"memory": {"kind": "episodic", "score": 7}})),
            ),
        )
        .expect("unscoped JSON doc inserts");
    let scoped_semantic = mutator
        .create_node(
            LabelSet::from_iter([doc.clone(), scope.clone()]),
            props(
                &payload,
                json(serde_json::json!({"memory": {"kind": "semantic", "score": 4}})),
            ),
        )
        .expect("scoped semantic doc inserts");
    mutator
        .create_node(
            LabelSet::from_iter([doc.clone(), scope.clone()]),
            props(&payload, Value::String(db_string("not json"))),
        )
        .expect("scoped non-JSON doc inserts");
    let scoped_array = mutator
        .create_node(
            LabelSet::from_iter([doc.clone(), scope.clone()]),
            props(
                &payload,
                json(serde_json::json!(["agent", {"memory": {"kind": "episodic"}}])),
            ),
        )
        .expect("scoped array doc inserts");
    mutator
        .create_node(
            LabelSet::from_iter([other, scope]),
            props(
                &payload,
                json(serde_json::json!({"memory": {"kind": "episodic", "score": 99}})),
            ),
        )
        .expect("wrong-label scoped doc inserts");
    txn.commit().expect("seed commits");

    SeedIds {
        scoped_semantic,
        scoped_array,
    }
}

#[test]
fn json_contains_candidate_nodes_filters_gql_candidate_set() {
    let graph = graph(514_201);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_json_candidate_docs(&graph);
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "MATCH (candidate:JsonScope) \
         WITH collect_list(candidate) AS nodes \
         CALL selene.json_contains_candidate_nodes( \
           'JsonDoc', \
           'payload', \
           json('{\"memory\":{\"kind\":\"episodic\"}}'), \
           nodes, \
           10 \
         ) YIELD node_id \
         RETURN node_id",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids.scoped_array]);
}

#[test]
fn json_path_candidate_nodes_match_path_semantics() {
    let graph = graph(514_202);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_json_candidate_docs(&graph);
    let mut session = Session::new(&graph);

    let path_exists = execute_rows(
        &mut session,
        "MATCH (candidate:JsonScope) \
         WITH collect_list(candidate) AS nodes \
         CALL selene.json_path_exists_candidate_nodes( \
           'JsonDoc', \
           'payload', \
           json_array('memory', 'score'), \
           nodes, \
           10 \
         ) YIELD node_id \
         RETURN node_id",
        &registry,
    );
    assert_eq!(
        node_column(&path_exists, "node_id"),
        vec![ids.scoped_semantic]
    );

    let path_contains = execute_rows(
        &mut session,
        "MATCH (candidate:JsonScope) \
         WITH collect_list(candidate) AS nodes \
         CALL selene.json_path_contains_candidate_nodes( \
           'JsonDoc', \
           'payload', \
           json_array(1, 'memory'), \
           json('{\"kind\":\"episodic\"}'), \
           nodes, \
           10 \
         ) YIELD node_id \
         RETURN node_id",
        &registry,
    );
    assert_eq!(
        node_column(&path_contains, "node_id"),
        vec![ids.scoped_array]
    );
}

#[test]
fn json_path_value_candidate_nodes_return_selected_values() {
    let graph = graph(514_203);
    let registry = BuiltinProcedureRegistry::new();
    let ids = seed_json_candidate_docs(&graph);
    let mut session = Session::new(&graph);

    let table = execute_rows(
        &mut session,
        "MATCH (candidate:JsonScope) \
         WITH collect_list(candidate) AS nodes \
         CALL selene.json_path_value_candidate_nodes( \
           'JsonDoc', \
           'payload', \
           json_array('memory', 'kind'), \
           nodes, \
           10 \
         ) YIELD node_id, value \
         RETURN node_id, value",
        &registry,
    );

    assert_eq!(node_column(&table, "node_id"), vec![ids.scoped_semantic]);
    assert_eq!(
        json_column(&table, "value"),
        vec![serde_json::json!("semantic")]
    );
}

#[test]
fn json_candidate_metadata_exposes_node_candidate_arguments() {
    let registry = BuiltinProcedureRegistry::new();
    let node_list_type = GqlType::List(Box::new(GqlType::NodeRef));
    for (procedure, arity, nodes_index) in [
        ("json_contains_candidate_nodes", 5, 3),
        ("json_path_exists_candidate_nodes", 5, 3),
        ("json_path_contains_candidate_nodes", 6, 4),
        ("json_path_value_candidate_nodes", 5, 3),
    ] {
        let name = [db_string("selene"), db_string(procedure)];
        let metadata = registry.lookup(&name).expect("JSON candidate resolves");
        let actual_arity = metadata.signature.arity();
        assert_eq!(actual_arity.minimum, arity, "{procedure}");
        assert_eq!(actual_arity.maximum, arity, "{procedure}");
        let parameter = &metadata.signature.parameters[nodes_index];
        assert_eq!(parameter.name.as_str(), "nodes", "{procedure}");
        assert_eq!(parameter.ty, node_list_type, "{procedure}");
    }

    let name = [
        db_string("selene"),
        db_string("json_path_value_candidate_nodes"),
    ];
    let metadata = registry
        .lookup(&name)
        .expect("json_path_value_candidate_nodes resolves");
    assert_eq!(metadata.output_schema.columns.len(), 2);
    assert_eq!(metadata.output_schema.columns[1].name.as_str(), "value");
    assert_eq!(metadata.output_schema.columns[1].ty, GqlType::Json);
}

#[test]
fn json_candidate_nodes_reject_non_node_candidates() {
    let graph = graph(514_204);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("nodes"), Value::List(vec![Value::Int(1)]));

    let err = session
        .execute_source(
            "CALL selene.json_contains_candidate_nodes( \
             'JsonDoc', \
             'payload', \
             json('{}'), \
             $nodes, \
             10 \
           )",
            &registry,
        )
        .expect_err("non-node candidates must fail");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("nodes[0] must be a NODE")
    ));
}
