//! Executor expression evaluator tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::empty_graph_context;
use selene_core::{Value, intern};
use selene_gql::{
    BinaryOp, Binding, BindingTableSchema, ExecutorError, ImplDefinedCaps, Literal, SourceSpan,
    UnaryOp, ValueExpr,
};

fn span() -> SourceSpan {
    SourceSpan::new(0, 1)
}

fn lit(literal: Literal) -> ValueExpr {
    ValueExpr::Literal(literal)
}

fn bool_lit(value: bool) -> ValueExpr {
    lit(Literal::Bool(value, span()))
}

fn null_lit() -> ValueExpr {
    lit(Literal::Null(span()))
}

fn eval(expr: &ValueExpr) -> Value {
    let caps = ImplDefinedCaps::default();
    let ctx = empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(
        expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
    .expect("expression evaluates")
}

#[test]
fn null_equality_propagates_unknown() {
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Eq,
        lhs: Box::new(null_lit()),
        rhs: Box::new(null_lit()),
        span: span(),
    };

    assert_eq!(eval(&expr), Value::Null);
}

#[test]
fn three_valued_and_or_not_truth_table() {
    let cases = [
        (
            BinaryOp::And,
            bool_lit(false),
            null_lit(),
            Value::Bool(false),
        ),
        (BinaryOp::And, bool_lit(true), null_lit(), Value::Null),
        (BinaryOp::Or, bool_lit(true), null_lit(), Value::Bool(true)),
        (BinaryOp::Or, bool_lit(false), null_lit(), Value::Null),
    ];

    for (op, lhs, rhs, expected) in cases {
        let expr = ValueExpr::BinaryOp {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: span(),
        };
        assert_eq!(eval(&expr), expected);
    }

    let not_null = ValueExpr::UnaryOp {
        op: UnaryOp::Not,
        operand: Box::new(null_lit()),
        span: span(),
    };
    assert_eq!(eval(&not_null), Value::Null);
}

#[test]
fn arithmetic_overflow_is_data_exception() {
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Add,
        lhs: Box::new(lit(Literal::Integer(i64::MAX, span()))),
        rhs: Box::new(lit(Literal::Integer(1, span()))),
        span: span(),
    };
    let caps = ImplDefinedCaps::default();
    let ctx = empty_graph_context(&caps);
    let err = selene_gql::runtime::evaluate_for_test(
        &expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
    .expect_err("overflow errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22000");
}

#[test]
fn function_call_is_explicitly_unsupported() {
    let expr = ValueExpr::FunctionCall {
        name: vec![intern("unsupported").unwrap()],
        args: Vec::new(),
        star: false,
        distinct: false,
        span: span(),
    };
    let caps = ImplDefinedCaps::default();
    let ctx = empty_graph_context(&caps);
    let err = selene_gql::runtime::evaluate_for_test(
        &expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
    .expect_err("function call unsupported");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "function call evaluation not implemented"
        }
    ));
}
