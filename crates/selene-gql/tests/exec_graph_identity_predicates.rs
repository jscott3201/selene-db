//! Graph identity predicate evaluator tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{db_string, empty_graph_context};
use selene_core::{EdgeId, NodeId, Value};
use selene_gql::{
    AnalyzedType, Binding, BindingTableColumn, BindingTableSchema, ExecutorError, ImplDefinedCaps,
    Literal, SourceSpan, ValueExpr,
};

fn span() -> SourceSpan {
    SourceSpan::new(0, 1)
}

fn var(name: selene_core::DbString) -> ValueExpr {
    ValueExpr::Variable { name, span: span() }
}

fn all_different(items: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::AllDifferent {
        items,
        span: span(),
    }
}

fn same(items: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::Same {
        items,
        span: span(),
    }
}

fn named_column(name: selene_core::DbString) -> BindingTableColumn {
    BindingTableColumn {
        name: Some(name),
        hidden: None,
        ty: AnalyzedType::Dynamic,
    }
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

#[test]
fn all_different_compares_graph_reference_identity() {
    let left = db_string("left");
    let right = db_string("right");
    let expr = all_different(vec![var(left.clone()), var(right.clone())]);
    let schema = BindingTableSchema {
        columns: vec![named_column(left.clone()), named_column(right.clone())],
    };

    assert_eq!(
        eval_with_binding(
            &expr,
            &Binding::new([
                Value::NodeRef(NodeId::new(7)),
                Value::NodeRef(NodeId::new(8)),
            ]),
            &schema,
        )
        .expect("distinct nodes compare"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_binding(
            &expr,
            &Binding::new([
                Value::NodeRef(NodeId::new(7)),
                Value::NodeRef(NodeId::new(7)),
            ]),
            &schema,
        )
        .expect("same node compares"),
        Value::Bool(false)
    );
}

#[test]
fn same_compares_graph_reference_identity() {
    let left = db_string("left");
    let right = db_string("right");
    let expr = same(vec![var(left.clone()), var(right.clone())]);
    let schema = BindingTableSchema {
        columns: vec![named_column(left.clone()), named_column(right.clone())],
    };

    assert_eq!(
        eval_with_binding(
            &expr,
            &Binding::new([
                Value::EdgeRef(EdgeId::new(3)),
                Value::EdgeRef(EdgeId::new(3)),
            ]),
            &schema,
        )
        .expect("same edge compares"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_binding(
            &expr,
            &Binding::new([
                Value::EdgeRef(EdgeId::new(3)),
                Value::EdgeRef(EdgeId::new(4)),
            ]),
            &schema,
        )
        .expect("different edges compare"),
        Value::Bool(false)
    );
}

#[test]
fn graph_identity_predicates_reject_null_operands() {
    let left = db_string("left");
    let right = db_string("right");
    let schema = BindingTableSchema {
        columns: vec![named_column(left.clone()), named_column(right.clone())],
    };

    for expr in [
        all_different(vec![var(left.clone()), var(right.clone())]),
        same(vec![var(left.clone()), var(right.clone())]),
    ] {
        let err = eval_with_binding(
            &expr,
            &Binding::new([Value::NodeRef(NodeId::new(7)), Value::Null]),
            &schema,
        )
        .expect_err("NULL graph predicate operand is a data exception");
        assert!(matches!(err, ExecutorError::DataException { .. }));
        assert_eq!(err.gqlstatus().as_str(), "22004");
    }
}

#[test]
fn graph_identity_predicates_reject_mixed_reference_families() {
    let left = db_string("left");
    let right = db_string("right");
    let schema = BindingTableSchema {
        columns: vec![named_column(left.clone()), named_column(right.clone())],
    };

    for expr in [
        all_different(vec![var(left.clone()), var(right.clone())]),
        same(vec![var(left.clone()), var(right.clone())]),
    ] {
        let err = eval_with_binding(
            &expr,
            &Binding::new([
                Value::NodeRef(NodeId::new(7)),
                Value::EdgeRef(EdgeId::new(3)),
            ]),
            &schema,
        )
        .expect_err("node and edge references are not comparable");
        assert!(matches!(err, ExecutorError::DataException { .. }));
        assert_eq!(err.gqlstatus().as_str(), "22G04");
    }
}

#[test]
fn graph_identity_predicates_reject_non_reference_operands() {
    let left = db_string("left");
    let right = db_string("right");
    let schema = BindingTableSchema {
        columns: vec![named_column(left.clone()), named_column(right.clone())],
    };

    for expr in [
        all_different(vec![var(left.clone()), var(right.clone())]),
        same(vec![var(left.clone()), var(right.clone())]),
    ] {
        let err = eval_with_binding(
            &expr,
            &Binding::new([Value::NodeRef(NodeId::new(7)), Value::Int(1)]),
            &schema,
        )
        .expect_err("scalar graph predicate operand is a data exception");
        assert!(matches!(err, ExecutorError::DataException { .. }));
        assert_eq!(err.gqlstatus().as_str(), "22G03");
    }
}

#[test]
fn graph_identity_predicates_reject_literal_null_operands() {
    for expr in [
        all_different(vec![ValueExpr::Literal(Literal::Null(span()))]),
        same(vec![ValueExpr::Literal(Literal::Null(span()))]),
    ] {
        let err = eval_with_binding(
            &expr,
            &Binding::empty(),
            &BindingTableSchema { columns: vec![] },
        )
        .expect_err("literal NULL graph predicate operand is a data exception");
        assert!(matches!(err, ExecutorError::DataException { .. }));
        assert_eq!(err.gqlstatus().as_str(), "22004");
    }
}
