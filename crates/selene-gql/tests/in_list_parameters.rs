//! Parameterized `IN` predicate integration tests.

use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value};
use selene_gql::{
    AnalysisError, EmptyProcedureRegistry, ExpectedType, PipelineStatement, Session,
    StatementOutput, TypeMismatchContext, ValueExpr, analyze, parse,
};
use selene_graph::SharedGraph;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn props<const N: usize>(pairs: [(DbString, Value); N]) -> PropertyMap {
    PropertyMap::from_pairs(pairs).expect("test properties fit caps")
}

fn rows(output: StatementOutput) -> selene_gql::BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("write returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn single_value(session: &mut Session<'_>, source: &str) -> Value {
    let table = rows(
        session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect("query succeeds"),
    );
    assert_eq!(table.row_count(), 1);
    table.rows()[0].values()[0].clone()
}

fn list(values: impl IntoIterator<Item = Value>) -> Value {
    Value::List(values.into_iter().collect())
}

#[test]
fn parses_parameter_rhs_as_list_expression() {
    let statement = parse("RETURN 2 IN $ids AS hit").expect("query parses");
    let selene_gql::Statement::Query(pipeline) = statement else {
        panic!("expected query");
    };
    let PipelineStatement::Return(clause) = &pipeline.statements[0] else {
        panic!("expected RETURN");
    };
    let ValueExpr::InListExpression { operand, list, .. } = &clause.items[0].expr else {
        panic!("expected dynamic IN-list expression");
    };
    assert!(matches!(operand.as_ref(), ValueExpr::Literal(_)));
    assert!(matches!(list.as_ref(), ValueExpr::Parameter { .. }));
}

#[test]
fn typed_list_rhs_analyzes_when_element_type_matches() {
    analyze(
        parse("RETURN 'a' IN $items :: LIST<STRING> AS hit").expect("query parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect("typed list parameter analyzes");
}

#[test]
fn statically_known_non_list_rhs_is_rejected() {
    let err = analyze(
        parse("RETURN 1 IN 2 AS hit").expect("query parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("non-list RHS rejects");
    let AnalysisError::TypeMismatch {
        context, expected, ..
    } = err
    else {
        panic!("expected type mismatch, got {err:?}");
    };
    assert_eq!(context, TypeMismatchContext::InListUnification);
    assert_eq!(expected, ExpectedType::List);
}

#[test]
fn typed_list_rhs_rejects_incompatible_element_type() {
    let err = analyze(
        parse("RETURN 1 IN $items :: LIST<STRING> AS hit").expect("query parses"),
        &EmptyProcedureRegistry,
        None,
    )
    .expect_err("incompatible list item rejects");
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::InListUnification,
            ..
        }
    ));
}

#[test]
fn parameter_rhs_uses_in_list_three_valued_semantics() {
    let graph = SharedGraph::new(GraphId::new(59_001));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string("ids"), list([Value::Int(1), Value::Int(2)]));
    assert_eq!(
        single_value(&mut session, "RETURN 2 IN $ids AS hit"),
        Value::Bool(true)
    );

    session.bind_parameter(db_string("ids"), list([Value::Int(1), Value::Int(3)]));
    assert_eq!(
        single_value(&mut session, "RETURN 2 NOT IN $ids AS hit"),
        Value::Bool(true)
    );

    session.bind_parameter(db_string("ids"), list([Value::Int(1), Value::Null]));
    assert_eq!(
        single_value(&mut session, "RETURN 2 IN $ids AS hit"),
        Value::Null
    );

    session.bind_parameter(db_string("ids"), Value::Null);
    assert_eq!(
        single_value(&mut session, "RETURN 2 IN $ids AS hit"),
        Value::Null
    );
}

#[test]
fn literal_rhs_still_short_circuits_after_first_match() {
    let graph = SharedGraph::new(GraphId::new(59_003));
    let mut session = Session::new(&graph);
    assert_eq!(
        single_value(&mut session, "RETURN 1 IN [1, 1 / 0] AS hit"),
        Value::Bool(true)
    );
}

#[test]
fn match_where_accepts_parameterized_list_rhs() {
    let graph = SharedGraph::new(GraphId::new(59_002));
    let fact = db_string("Fact");
    let name = db_string("name");
    let kind = db_string("kind");
    {
        let mut txn = graph.begin_write();
        {
            let mut mutator = txn.mutator();
            for (node_name, node_kind) in [
                ("alpha", "episodic"),
                ("beta", "procedural"),
                ("gamma", "semantic"),
            ] {
                mutator
                    .create_node(
                        LabelSet::single(fact.clone()),
                        props([
                            (name.clone(), Value::String(db_string(node_name))),
                            (kind.clone(), Value::String(db_string(node_kind))),
                        ]),
                    )
                    .expect("node inserts");
            }
        }
        txn.commit().expect("fixture commits");
    }

    let mut session = Session::new(&graph);
    session.bind_parameter(
        db_string("kinds"),
        list([
            Value::String(db_string("episodic")),
            Value::String(db_string("semantic")),
        ]),
    );
    let table = rows(
        session
            .execute_source(
                "MATCH (n:Fact) WHERE n.kind IN $kinds RETURN n.name AS name ORDER BY name",
                &EmptyProcedureRegistry,
            )
            .expect("query succeeds"),
    );
    let names = table
        .rows()
        .iter()
        .map(|row| match &row.values()[0] {
            Value::String(value) => value.as_str().to_owned(),
            other => panic!("expected string, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["alpha", "gamma"]);
}
