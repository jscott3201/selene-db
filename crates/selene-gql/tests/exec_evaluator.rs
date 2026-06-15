//! Executor expression evaluator tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::empty_graph_context;
use selene_core::{Value, db_string};
use selene_gql::{
    AnalyzedType, BinaryOp, Binding, BindingTableColumn, BindingTableSchema, ExecutorError,
    FloatLiteralKind, GqlType, ImplDefinedCaps, IsCheckKind, Literal, NonEmpty, RecordType,
    SourceSpan, TruthValue, UnaryOp, ValueExpr, ast::CharacterStringLiteralKind,
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
    lit(Literal::Float(
        value,
        span(),
        FloatLiteralKind::CommonOrIntegerWithDoubleSuffix,
    ))
}

fn var(name: selene_core::DbString) -> ValueExpr {
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

fn named_column(name: selene_core::DbString) -> BindingTableColumn {
    BindingTableColumn {
        name: Some(name),
        hidden: None,
        ty: AnalyzedType::Dynamic,
    }
}

fn eval_binary_values(op: BinaryOp, lhs: Value, rhs: Value) -> Result<Value, ExecutorError> {
    let lhs_name = db_string("lhs").unwrap();
    let rhs_name = db_string("rhs").unwrap();
    let expr = ValueExpr::BinaryOp {
        op,
        lhs: Box::new(var(lhs_name.clone())),
        rhs: Box::new(var(rhs_name.clone())),
        span: span(),
    };
    let binding = Binding::new([lhs, rhs]);
    let schema = BindingTableSchema {
        columns: vec![named_column(lhs_name), named_column(rhs_name)],
    };
    eval_with_binding(&expr, &binding, &schema)
}

#[path = "exec_evaluator/records_runtime.rs"]
mod records_runtime;
#[path = "exec_evaluator/scalar_ops.rs"]
mod scalar_ops;
