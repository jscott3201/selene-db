//! Executor expression evaluator tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::empty_graph_context;
use selene_core::{Value, intern};
use selene_gql::{
    AnalyzedType, BinaryOp, Binding, BindingTableColumn, BindingTableSchema, ExecutorError,
    ImplDefinedCaps, Literal, NonEmpty, SourceSpan, UnaryOp, ValueExpr,
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

fn int_lit(value: i64) -> ValueExpr {
    lit(Literal::Integer(value, span()))
}

fn float_lit(value: f64) -> ValueExpr {
    lit(Literal::Float(value, span()))
}

fn var(name: selene_core::IStr) -> ValueExpr {
    ValueExpr::Variable { name, span: span() }
}

fn eval_result(expr: &ValueExpr) -> Result<Value, ExecutorError> {
    let caps = ImplDefinedCaps::default();
    let ctx = empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(
        expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
}

fn eval(expr: &ValueExpr) -> Value {
    eval_result(expr).expect("expression evaluates")
}

fn eval_with_binding(
    expr: &ValueExpr,
    binding: &Binding,
    schema: &BindingTableSchema,
) -> Result<Value, ExecutorError> {
    let caps = ImplDefinedCaps::default();
    let ctx = empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(expr, binding, schema, &ctx)
}

fn named_column(name: selene_core::IStr) -> BindingTableColumn {
    BindingTableColumn {
        name: Some(name),
        hidden: None,
        ty: AnalyzedType::Dynamic,
    }
}

fn eval_binary_values(op: BinaryOp, lhs: Value, rhs: Value) -> Result<Value, ExecutorError> {
    let lhs_name = intern("lhs").unwrap();
    let rhs_name = intern("rhs").unwrap();
    let expr = ValueExpr::BinaryOp {
        op,
        lhs: Box::new(var(lhs_name)),
        rhs: Box::new(var(rhs_name)),
        span: span(),
    };
    let binding = Binding::new([lhs, rhs]);
    let schema = BindingTableSchema {
        columns: vec![named_column(lhs_name), named_column(rhs_name)],
    };
    eval_with_binding(&expr, &binding, &schema)
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
fn numeric_equal_top_level_float_nan_returns_null() {
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Eq,
        lhs: Box::new(float_lit(f64::NAN)),
        rhs: Box::new(float_lit(f64::NAN)),
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
fn eval_arithmetic_uint_uint_no_overflow_returns_uint() {
    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Uint(10), Value::Uint(20))
            .expect("uint arithmetic succeeds"),
        Value::Uint(30)
    );
}

#[test]
fn eval_arithmetic_uint_uint_overflow_widens_to_int128() {
    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Uint(u64::MAX), Value::Uint(1))
            .expect("uint overflow widens"),
        Value::Int128(i128::from(u64::MAX) + 1)
    );
}

#[test]
fn eval_arithmetic_int_uint_returns_int128() {
    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Int(-1), Value::Uint(2))
            .expect("mixed arithmetic widens"),
        Value::Int128(1)
    );
}

#[test]
fn eval_arithmetic_uint_int_returns_int128() {
    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Uint(2), Value::Int(-1))
            .expect("mixed arithmetic widens"),
        Value::Int128(1)
    );
}

#[test]
fn eval_arithmetic_int128_int_chains_exactly() {
    let widened = eval_binary_values(BinaryOp::Add, Value::Uint(u64::MAX), Value::Uint(1))
        .expect("uint overflow widens");

    assert_eq!(
        eval_binary_values(BinaryOp::Add, widened, Value::Int(1))
            .expect("chained Int128 arithmetic succeeds"),
        Value::Int128(i128::from(u64::MAX) + 2)
    );
}

#[test]
fn eval_arithmetic_int128_int128_succeeds() {
    let half = i128::MAX / 2;

    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Int128(half), Value::Int128(half))
            .expect("Int128 arithmetic succeeds"),
        Value::Int128(half + half)
    );
}

#[test]
fn eval_arithmetic_int_int128_returns_int128() {
    assert_eq!(
        eval_binary_values(BinaryOp::Add, Value::Int(5), Value::Int128(7))
            .expect("Int + Int128 arithmetic succeeds"),
        Value::Int128(12)
    );
}

#[test]
fn eval_arithmetic_int128_overflow_returns_data_exception() {
    let err = eval_binary_values(BinaryOp::Mul, Value::Uint(u64::MAX), Value::Uint(u64::MAX))
        .expect_err("i128 widening can still overflow");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22000");
}

#[test]
fn large_integer_equality_is_exact() {
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Eq,
        lhs: Box::new(int_lit(9_007_199_254_740_992)),
        rhs: Box::new(int_lit(9_007_199_254_740_993)),
        span: span(),
    };

    assert_eq!(eval(&expr), Value::Bool(false));
}

#[test]
fn large_integer_in_list_is_exact() {
    let expr = ValueExpr::InList {
        operand: Box::new(int_lit(9_007_199_254_740_993)),
        list: vec![int_lit(9_007_199_254_740_992)],
        negated: false,
        span: span(),
    };

    assert_eq!(eval(&expr), Value::Bool(false));
}

#[test]
fn lossy_integer_float_ordering_is_data_exception() {
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Gt,
        lhs: Box::new(int_lit(9_007_199_254_740_993)),
        rhs: Box::new(float_lit(9_007_199_254_740_992.0)),
        span: span(),
    };
    let err = eval_result(&expr).expect_err("lossy comparison errors");

    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22000");
}

#[test]
fn unknown_scalar_function_returns_22g03() {
    let expr = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![intern("unsupported").unwrap()]).expect("non-empty"),
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
    .expect_err("unknown function errors");

    assert!(matches!(err, ExecutorError::UnknownFunction { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}
