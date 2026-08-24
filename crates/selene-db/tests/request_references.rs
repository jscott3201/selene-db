//! Request preflight coverage for graph-backed parameter references.

use selene_core::{
    BindingTableId, EdgeDirection, EdgeId, GraphId as ValueGraphId, NodeId, Path as ValuePath,
    PathSegment as ValuePathSegment, Record, RecordTypeId, RecordTyped, Value, db_string,
};
use selene_db::{
    CreatePolicy, Database, GeneralParameter, GqlType, ObjectPath, Request, RequestOutcome,
    RequestParams, SchemaPath,
};
use selene_gql::BindingTableType;

fn fixture(name: &str) -> selene_db::Session {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema = SchemaPath::regular("selene", name).unwrap();
    let graph = ObjectPath::regular("selene", name, "main").unwrap();
    catalog
        .create_schema(&schema, CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&graph, None, CreatePolicy::Strict)
        .unwrap();
    database.session(&graph).unwrap()
}

fn graph_id(session: &selene_db::Session) -> ValueGraphId {
    ValueGraphId::new(session.context().dependencies().current_graph().get())
}

fn execute_parameter(
    session: &selene_db::Session,
    declared_type: GqlType,
    value: Value,
) -> RequestOutcome {
    let mut params = RequestParams::new();
    params
        .insert(
            "value",
            GeneralParameter::new(declared_type, value).unwrap(),
        )
        .unwrap();
    session.execute_request(Request::with_params("RETURN $value", params))
}

fn assert_invalid_reference(outcome: &RequestOutcome, detail: &str) {
    let error = outcome.error().expect("reference must be rejected");
    assert_eq!(error.gqlstatus().unwrap().as_str(), "42002");
    assert!(error.message().contains(detail), "{}", error.message());
}

#[test]
fn graph_and_node_references_must_belong_to_the_live_selected_graph() {
    let session = fixture("reference_nodes");
    let graph = graph_id(&session);
    assert!(matches!(
        execute_parameter(&session, GqlType::GraphRef, Value::GraphRef(graph)),
        RequestOutcome::Succeeded { .. }
    ));
    assert_invalid_reference(
        &execute_parameter(
            &session,
            GqlType::GraphRef,
            Value::GraphRef(ValueGraphId::new(graph.get() + 1)),
        ),
        "another graph",
    );

    session.execute("INSERT (:Live) FINISH").unwrap();
    assert!(matches!(
        execute_parameter(&session, GqlType::NodeRef, Value::NodeRef(NodeId::new(1))),
        RequestOutcome::Succeeded { .. }
    ));
    session.execute("MATCH (n:Live) DELETE n FINISH").unwrap();
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::NodeRef, Value::NodeRef(NodeId::new(1))),
        "not alive",
    );
}

#[test]
fn edge_and_path_references_validate_elements_connectivity_and_staleness() {
    let session = fixture("reference_paths");
    session.execute("INSERT (:A)-[:STEP]->(:B) FINISH").unwrap();
    let path = ValuePath {
        graph: graph_id(&session),
        start: NodeId::new(1),
        segments: [ValuePathSegment {
            edge: EdgeId::new(1),
            direction: EdgeDirection::Outgoing,
            node: NodeId::new(2),
        }]
        .into_iter()
        .collect(),
    };
    assert!(matches!(
        execute_parameter(&session, GqlType::EdgeRef, Value::EdgeRef(EdgeId::new(1))),
        RequestOutcome::Succeeded { .. }
    ));
    assert!(matches!(
        execute_parameter(&session, GqlType::Path, Value::Path(Box::new(path.clone()))),
        RequestOutcome::Succeeded { .. }
    ));

    let foreign_path = ValuePath {
        graph: ValueGraphId::new(graph_id(&session).get() + 1),
        ..path.clone()
    };
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Path, Value::Path(Box::new(foreign_path))),
        "another graph",
    );

    let disconnected = ValuePath {
        start: NodeId::new(2),
        ..path.clone()
    };
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Path, Value::Path(Box::new(disconnected))),
        "not connected",
    );

    session
        .execute("MATCH ()-[e:STEP]->() DELETE e FINISH")
        .unwrap();
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::EdgeRef, Value::EdgeRef(EdgeId::new(1))),
        "not alive",
    );
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Path, Value::Path(Box::new(path))),
        "stale element",
    );
}

#[test]
fn nested_references_are_walked_and_table_references_are_deferred() {
    let session = fixture("reference_nested");
    let stale = Value::NodeRef(NodeId::new(99));

    let list = Value::List(vec![Value::List(vec![stale.clone()])]);
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Any, list),
        "not alive",
    );

    let record = Value::Record(Box::new(Record::Open(
        [(db_string("nested").unwrap(), stale.clone())]
            .into_iter()
            .collect(),
    )));
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Any, record),
        "not alive",
    );

    let typed = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(stale)].into_iter().collect(),
    }));
    assert_invalid_reference(
        &execute_parameter(&session, GqlType::Any, typed),
        "not alive",
    );

    let table = execute_parameter(
        &session,
        GqlType::TableRef(BindingTableType::Any),
        Value::TableRef(BindingTableId::new(1)),
    );
    assert_invalid_reference(&table, "table parameters are pending M03-PR03");
}
