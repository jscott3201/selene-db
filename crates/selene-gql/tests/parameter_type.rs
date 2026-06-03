//! Typed parameter runtime validation regressions.

use selene_core::{GraphId, IStr, Record, Value, VectorValue, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlType, PipelineStatement, QueryPipeline, RecordType,
    ReturnClause, ReturnItem, Session, SourceSpan, Statement, ValueExpr, analyze,
    execute_statement, plan,
};
use selene_graph::SharedGraph;
use smallvec::smallvec;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn typed_parameter_statement(name: IStr, declared_type: GqlType) -> Statement {
    let span = SourceSpan::new(0, 4);
    Statement::Query(QueryPipeline {
        statements: vec![PipelineStatement::Return(ReturnClause {
            distinct: false,
            star: false,
            items: vec![ReturnItem {
                expr: ValueExpr::Parameter {
                    name: name.clone(),
                    declared_type: Some(declared_type),
                    span,
                },
                alias: Some(name),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    })
}

#[test]
fn typed_parameter_runtime_rejects_mismatched_list_elements() {
    let graph = SharedGraph::new(GraphId::new(4122));
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("vals"),
        Value::List(vec![Value::Int(1), Value::String(istr("bad"))]),
    );

    let err = session
        .execute_source("RETURN $vals :: LIST<INT> AS vals", &EmptyProcedureRegistry)
        .expect_err("typed list parameter rejects mismatched element");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name,
            ref expected,
            actual: "LIST",
            ..
        } if name.as_str() == "vals" && expected == "LIST<INTEGER>"
    ));
}

#[test]
fn typed_parameter_runtime_accepts_vector() {
    let name = istr("vec");
    let statement = typed_parameter_statement(name.clone(), GqlType::Vector);
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(4125));
    let mut session = Session::new(&graph);

    session.bind_parameter(
        name,
        Value::Vector(VectorValue::new(vec![1.0, 2.0]).expect("test vector is valid")),
    );
    execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect("vector parameter accepts matching value");
}

#[test]
fn typed_parameter_runtime_rejects_non_vector() {
    let name = istr("vec");
    let statement = typed_parameter_statement(name.clone(), GqlType::Vector);
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(4126));
    let mut session = Session::new(&graph);

    session.bind_parameter(name.clone(), Value::List(vec![Value::Float(1.0)]));
    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("vector parameter rejects mismatched value");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name: err_name,
            ref expected,
            actual: "LIST",
            ..
        } if err_name == name && expected == "VECTOR"
    ));
}

#[test]
fn typed_parameter_runtime_rejects_mismatched_closed_record_fields() {
    let span = SourceSpan::new(0, 4);
    let field = istr("count");
    let name = istr("rec");
    let statement = Statement::Query(QueryPipeline {
        statements: vec![PipelineStatement::Return(ReturnClause {
            distinct: false,
            star: false,
            items: vec![ReturnItem {
                expr: ValueExpr::Parameter {
                    name: name.clone(),
                    declared_type: Some(GqlType::Record(RecordType::Closed(vec![(
                        field.clone(),
                        GqlType::Integer,
                    )]))),
                    span,
                },
                alias: Some(name.clone()),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    });
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(4123));
    let mut session = Session::new(&graph);

    session.bind_parameter(
        name.clone(),
        Value::Record(Box::new(Record::Open(smallvec![(
            field,
            Value::String(istr("three")),
        )]))),
    );
    let err = execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect_err("closed record parameter rejects mismatched field type");
    assert!(matches!(
        err,
        ExecutorError::InvalidParameterType {
            name: err_name,
            ref expected,
            actual: "RECORD",
            ..
        } if err_name == name && expected == "RECORD"
    ));
}

#[test]
fn typed_parameter_runtime_accepts_matching_closed_record_fields() {
    let span = SourceSpan::new(0, 4);
    let field = istr("count");
    let name = istr("rec");
    let statement = Statement::Query(QueryPipeline {
        statements: vec![PipelineStatement::Return(ReturnClause {
            distinct: false,
            star: false,
            items: vec![ReturnItem {
                expr: ValueExpr::Parameter {
                    name: name.clone(),
                    declared_type: Some(GqlType::Record(RecordType::Closed(vec![(
                        field.clone(),
                        GqlType::Integer,
                    )]))),
                    span,
                },
                alias: Some(name.clone()),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    });
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("statement analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("statement plans");
    let graph = SharedGraph::new(GraphId::new(4124));
    let mut session = Session::new(&graph);

    session.bind_parameter(
        name,
        Value::Record(Box::new(Record::Open(smallvec![(field, Value::Int(3))]))),
    );
    execute_statement(&plan, &mut session, &EmptyProcedureRegistry)
        .expect("closed record parameter accepts matching field type");
}
