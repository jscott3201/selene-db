//! End-to-end coverage for additive `ALTER NODE TYPE`.

use selene_core::{GraphId, Value};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlStatus, Session, StatementOutput, WriteOutcome,
    analyze, execute_statement, parse, plan,
};
use selene_graph::{GraphTypeDef, SharedGraph};

fn db_string(value: &str) -> selene_core::DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn execute(source: &str, session: &mut Session<'_>) -> Result<StatementOutput, ExecutorError> {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    execute_statement(&plan, session, &EmptyProcedureRegistry)
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn written(output: StatementOutput) -> WriteOutcome {
    match output {
        StatementOutput::Written(outcome) => outcome,
        other => panic!("expected written output, got {other:?}"),
    }
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("alter.node.test.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

#[test]
fn alter_node_type_preserves_live_rows_and_defaults_only_future_inserts() {
    let graph = empty_closed_graph(107_301);
    let mut session = Session::new(&graph);
    execute(
        "CREATE NODE TYPE :Person (name STRING NOT NULL)",
        &mut session,
    )
    .expect("type creates");
    execute("INSERT (:Person {name: 'old'}) FINISH", &mut session).expect("old row inserts");

    execute(
        "ALTER NODE TYPE :Person (active BOOLEAN DEFAULT true)",
        &mut session,
    )
    .expect("type alters");
    execute("INSERT (:Person {name: 'new'}) FINISH", &mut session).expect("new row inserts");

    let table = rows(
        execute(
            "MATCH (n:Person) RETURN n.name AS name, n.active AS active ORDER BY name",
            &mut session,
        )
        .expect("rows query"),
    );
    assert_eq!(table.row_count(), 2);
    assert_eq!(
        table.rows()[0].values(),
        &[Value::String(db_string("new")), Value::Bool(true)]
    );
    assert_eq!(
        table.rows()[1].values(),
        &[Value::String(db_string("old")), Value::Null]
    );

    let show = rows(execute("SHOW NODE TYPES", &mut session).expect("show succeeds"));
    assert_eq!(
        show.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Person (name :: STRING NOT NULL, active :: BOOLEAN DEFAULT TRUE)"
        ))
    );
}

#[test]
fn exact_repeat_is_idempotent_but_conflicting_redefinition_rejects() {
    let graph = empty_closed_graph(107_302);
    let mut session = Session::new(&graph);
    execute("CREATE NODE TYPE :Person ()", &mut session).expect("type creates");
    let alter = "ALTER NODE TYPE :Person (note STRING DEFAULT 'new')";

    let first = written(execute(alter, &mut session).expect("first alter succeeds"));
    assert_eq!(first.changes.len(), 1);
    let second = written(execute(alter, &mut session).expect("exact repeat succeeds"));
    assert!(second.changes.is_empty(), "exact repeat must be a no-op");

    let error = execute("ALTER NODE TYPE :Person (note INTEGER)", &mut session)
        .expect_err("conflicting redefinition rejects");
    assert_eq!(error.gqlstatus(), GqlStatus::GRAPH_TYPE_VIOLATION);
    assert!(format!("{error:?}").contains("redefine property"));
}

#[test]
fn missing_type_and_open_graph_reject() {
    let closed = empty_closed_graph(107_303);
    let mut closed_session = Session::new(&closed);
    let missing = execute(
        "ALTER NODE TYPE :Missing (note STRING)",
        &mut closed_session,
    )
    .expect_err("missing type rejects");
    assert_eq!(missing.gqlstatus(), GqlStatus::GRAPH_TYPE_VIOLATION);
    assert!(format!("{missing:?}").contains("does not exist"));

    let open = SharedGraph::new(GraphId::new(107_304));
    let mut open_session = Session::new(&open);
    let error = execute("ALTER NODE TYPE :Person (note STRING)", &mut open_session)
        .expect_err("open graph rejects catalog type DDL");
    assert!(matches!(
        error,
        ExecutorError::GraphTypeViolation { message, .. }
            if message.contains("open graph (GG01) does not support catalog type DDL")
    ));
}

#[test]
fn required_and_inline_indexed_additions_reject() {
    let graph = empty_closed_graph(107_305);
    let mut session = Session::new(&graph);
    execute("CREATE NODE TYPE :Person ()", &mut session).expect("type creates");

    for source in [
        "ALTER NODE TYPE :Person (active BOOLEAN NOT NULL)",
        "ALTER NODE TYPE :Person (active BOOLEAN NOT NULL DEFAULT true)",
    ] {
        let error = execute(source, &mut session).expect_err("required addition rejects");
        assert_eq!(error.gqlstatus(), GqlStatus::GRAPH_TYPE_VIOLATION);
        assert!(format!("{error:?}").contains("cannot add required property"));
    }

    let indexed = execute(
        "ALTER NODE TYPE :Person (email STRING INDEXED)",
        &mut session,
    )
    .expect_err("inline index rejects");
    assert_eq!(indexed.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
    assert!(
        format!("{indexed:?}").contains("inline INDEXED on ALTER NODE TYPE properties"),
        "diagnostic must identify the node ALTER surface: {indexed:?}"
    );
}
