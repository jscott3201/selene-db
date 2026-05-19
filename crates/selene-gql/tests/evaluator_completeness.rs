//! BRIEF-116 evaluator completeness coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use std::sync::Arc;

use exec_common::{ExecFixture, column_values, execute_read, istr, planned};
use selene_core::{EdgeId, NodeId, Value, intern};
use selene_gql::{
    AnalyzedType, BinaryOp, Binding, BindingTableColumn, BindingTableSchema, ExecutorError,
    GqlType, IsCheckKind, LabelExpr, Literal, NonEmpty, NormalForm, SourceSpan, ValueExpr,
};

fn span() -> SourceSpan {
    SourceSpan::new(0, 1)
}

fn lit(literal: Literal) -> ValueExpr {
    ValueExpr::Literal(literal)
}

fn int_lit(value: i64) -> ValueExpr {
    lit(Literal::Integer(value, span()))
}

fn string_lit(value: &str) -> ValueExpr {
    lit(Literal::String(istr(value), span()))
}

fn null_lit() -> ValueExpr {
    lit(Literal::Null(span()))
}

fn bool_lit(value: bool) -> ValueExpr {
    lit(Literal::Bool(value, span()))
}

fn var(name: selene_core::IStr) -> ValueExpr {
    ValueExpr::Variable { name, span: span() }
}

fn named_column(name: selene_core::IStr) -> BindingTableColumn {
    BindingTableColumn {
        name: Some(name),
        hidden: None,
        ty: AnalyzedType::Dynamic,
    }
}

fn eval(expr: &ValueExpr) -> Result<Value, ExecutorError> {
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    selene_gql::runtime::evaluate_for_test(
        expr,
        &Binding::empty(),
        &BindingTableSchema { columns: vec![] },
        &ctx,
    )
}

fn eval_with_binding(
    expr: &ValueExpr,
    binding: Binding,
    columns: Vec<selene_core::IStr>,
) -> Result<Value, ExecutorError> {
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    let schema = BindingTableSchema {
        columns: columns.into_iter().map(named_column).collect(),
    };
    selene_gql::runtime::evaluate_for_test(expr, &binding, &schema, &ctx)
}

fn eval_with_fixture(
    expr: &ValueExpr,
    fixture: &ExecFixture,
    binding: Binding,
    columns: Vec<selene_core::IStr>,
) -> Result<Value, ExecutorError> {
    let plan = planned("RETURN 1 AS keepalive");
    let ctx = fixture.context_caps(&plan);
    let schema = BindingTableSchema {
        columns: columns.into_iter().map(named_column).collect(),
    };
    selene_gql::runtime::evaluate_for_test(expr, &binding, &schema, &ctx)
}

fn single_value(source: &str, column: &str) -> Value {
    let table = execute_read(source);
    let mut values = column_values(&table, column);
    assert_eq!(values.len(), 1);
    values.pop().expect("one row")
}

#[test]
fn scalar_numeric_functions_dispatch() {
    let table = execute_read(
        "RETURN abs(-3) AS abs_value, ceil(1.2) AS ceil_value, floor(1.8) AS floor_value, \
         round(1.6) AS round_value, mod(7, 4) AS mod_value, sqrt(9) AS sqrt_value, \
         power(2, 3) AS power_value",
    );

    assert_eq!(column_values(&table, "abs_value"), vec![Value::Int(3)]);
    assert_eq!(column_values(&table, "ceil_value"), vec![Value::Float(2.0)]);
    assert_eq!(
        column_values(&table, "floor_value"),
        vec![Value::Float(1.0)]
    );
    assert_eq!(
        column_values(&table, "round_value"),
        vec![Value::Float(2.0)]
    );
    assert_eq!(column_values(&table, "mod_value"), vec![Value::Int(3)]);
    assert_eq!(column_values(&table, "sqrt_value"), vec![Value::Float(3.0)]);
    assert_eq!(column_values(&table, "power_value"), vec![Value::Int(8)]);
}

#[test]
fn scalar_string_and_collection_functions_dispatch() {
    let table = execute_read(
        "RETURN length('abc') AS len, substring('abcdef', 2, 3) AS sub, upper('ab') AS up, \
         lower('AB') AS low, trim(' x ') AS trimmed, coalesce(null, 'x') AS co, \
         nullif('x', 'x') AS nf, size([1, 2, 3]) AS sz",
    );

    assert_eq!(column_values(&table, "len"), vec![Value::Int(3)]);
    assert_eq!(
        column_values(&table, "sub"),
        vec![Value::ExternalString(Arc::from("cde"))]
    );
    assert_eq!(
        column_values(&table, "up"),
        vec![Value::ExternalString(Arc::from("AB"))]
    );
    assert_eq!(
        column_values(&table, "low"),
        vec![Value::ExternalString(Arc::from("ab"))]
    );
    assert_eq!(
        column_values(&table, "trimmed"),
        vec![Value::ExternalString(Arc::from("x"))]
    );
    assert_eq!(column_values(&table, "co"), vec![Value::String(istr("x"))]);
    assert_eq!(column_values(&table, "nf"), vec![Value::Null]);
    assert_eq!(column_values(&table, "sz"), vec![Value::Int(3)]);
}

#[test]
fn scalar_function_errors_have_typed_statuses() {
    let unknown = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![intern("missing_fn").unwrap()]).expect("non-empty"),
        args: Vec::new(),
        star: false,
        distinct: false,
        span: span(),
    };
    let err = eval(&unknown).expect_err("unknown function errors");
    assert!(matches!(err, ExecutorError::UnknownFunction { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let bad_arity = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![intern("abs").unwrap()]).expect("non-empty"),
        args: Vec::new(),
        star: false,
        distinct: false,
        span: span(),
    };
    let err = eval(&bad_arity).expect_err("wrong arity errors");
    assert!(matches!(err, ExecutorError::FunctionArityMismatch { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let bad_modifier = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![intern("abs").unwrap()]).expect("non-empty"),
        args: vec![int_lit(1)],
        star: false,
        distinct: true,
        span: span(),
    };
    let err = eval(&bad_modifier).expect_err("DISTINCT is aggregate-only");
    assert!(matches!(err, ExecutorError::InvalidFunctionModifier { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn binary_operator_completion_covers_power_xor_concat_and_string_predicates() {
    let list_expr = ValueExpr::BinaryOp {
        op: BinaryOp::Concat,
        lhs: Box::new(ValueExpr::ListLiteral {
            items: vec![int_lit(1)],
            span: span(),
        }),
        rhs: Box::new(ValueExpr::ListLiteral {
            items: vec![int_lit(2)],
            span: span(),
        }),
        span: span(),
    };
    assert_eq!(
        eval(&list_expr).expect("list concat"),
        Value::List(vec![Value::Int(1), Value::Int(2)])
    );

    let table = execute_read(
        "RETURN true XOR false AS xor_value, 'ab' || 'cd' AS concat_value, \
         'abcdef' CONTAINS 'bcd' AS contains_value, \
         'abcdef' STARTS WITH 'abc' AS starts_value, \
         'abcdef' ENDS WITH 'def' AS ends_value",
    );
    assert_eq!(column_values(&table, "xor_value"), vec![Value::Bool(true)]);
    assert_eq!(
        column_values(&table, "concat_value"),
        vec![Value::ExternalString(Arc::from("abcd"))]
    );
    assert_eq!(
        column_values(&table, "contains_value"),
        vec![Value::Bool(true)]
    );
    assert_eq!(
        column_values(&table, "starts_value"),
        vec![Value::Bool(true)]
    );
    assert_eq!(column_values(&table, "ends_value"), vec![Value::Bool(true)]);
}

#[test]
fn predicate_completion_covers_like_between_all_different_and_same() {
    assert_eq!(
        single_value(
            "RETURN 'alphabet' LIKE 'a%bet' AS like_value, 5 BETWEEN 1 AND 10 AS between_value, \
             ALL_DIFFERENT(1, 2, 3) AS diff_value",
            "like_value",
        ),
        Value::Bool(true)
    );
    assert_eq!(
        single_value(
            "RETURN 'alphabet' LIKE 'a%bet' AS like_value, 5 BETWEEN 1 AND 10 AS between_value, \
             ALL_DIFFERENT(1, 2, 3) AS diff_value",
            "between_value",
        ),
        Value::Bool(true)
    );
    assert_eq!(
        single_value(
            "RETURN 'alphabet' LIKE 'a%bet' AS like_value, 5 BETWEEN 1 AND 10 AS between_value, \
             ALL_DIFFERENT(1, 2, 3) AS diff_value",
            "diff_value",
        ),
        Value::Bool(true)
    );

    let left = intern("left").unwrap();
    let right = intern("right").unwrap();
    let same = ValueExpr::Same {
        items: vec![var(left), var(right)],
        span: span(),
    };
    assert_eq!(
        eval_with_binding(
            &same,
            Binding::new([
                Value::NodeRef(NodeId::new(7)),
                Value::NodeRef(NodeId::new(7))
            ]),
            vec![left, right],
        )
        .expect("SAME evaluates"),
        Value::Bool(true)
    );
}

#[test]
fn is_predicates_and_property_exists_use_graph_snapshot() {
    let fixture = ExecFixture::build();
    let node = intern("node").unwrap();
    let edge = intern("edge").unwrap();

    let labeled = ValueExpr::IsCheck {
        operand: Box::new(var(node)),
        kind: IsCheckKind::Labeled(LabelExpr::Single(fixture.person)),
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &labeled,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            vec![node],
        )
        .expect("label predicate evaluates"),
        Value::Bool(true)
    );

    let property_exists = ValueExpr::PropertyExists {
        target: Box::new(var(node)),
        key: fixture.name,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            vec![node],
        )
        .expect("property exists evaluates"),
        Value::Bool(true)
    );

    let directed = ValueExpr::IsCheck {
        operand: Box::new(var(edge)),
        kind: IsCheckKind::Directed,
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &directed,
            &fixture,
            Binding::new([Value::EdgeRef(EdgeId::new(1))]),
            vec![edge],
        )
        .expect("directed predicate evaluates"),
        Value::Bool(true)
    );

    let source = ValueExpr::IsCheck {
        operand: Box::new(var(edge)),
        kind: IsCheckKind::SourceOf(Box::new(var(node))),
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &source,
            &fixture,
            Binding::new([
                Value::EdgeRef(EdgeId::new(1)),
                Value::NodeRef(NodeId::new(1))
            ]),
            vec![edge, node],
        )
        .expect("source predicate evaluates"),
        Value::Bool(true)
    );
}

#[test]
fn is_typed_and_is_normalized_scope_cut_are_explicit() {
    let typed = ValueExpr::IsCheck {
        operand: Box::new(string_lit("abc")),
        kind: IsCheckKind::Typed(GqlType::String),
        negated: false,
        span: span(),
    };
    assert_eq!(eval(&typed).expect("typed predicate"), Value::Bool(true));

    let normalized = ValueExpr::IsCheck {
        operand: Box::new(string_lit("abc")),
        kind: IsCheckKind::Normalized(NormalForm::Nfc),
        negated: false,
        span: span(),
    };
    let err = eval(&normalized).expect_err("normalization is v1.2");
    assert!(matches!(err, ExecutorError::FeatureNotInV1_1 { .. }));
    assert_eq!(err.gqlstatus().as_str(), "42N01");
}

#[test]
fn case_list_access_and_record_literal_evaluate() {
    assert_eq!(
        single_value(
            "RETURN CASE WHEN false THEN 1 WHEN true THEN 2 ELSE 3 END AS value",
            "value",
        ),
        Value::Int(2)
    );
    assert_eq!(
        single_value("RETURN [10, 20, 30][1] AS value", "value"),
        Value::Int(20)
    );
    assert_eq!(
        single_value("RETURN [10][-1] AS value", "value"),
        Value::Null
    );

    let record = ValueExpr::RecordLiteral {
        fields: vec![(istr("a"), int_lit(1)), (istr("b"), bool_lit(true))],
        span: span(),
    };
    match eval(&record).expect("record evaluates") {
        Value::Record(record) => match *record {
            selene_core::Record::Open(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0], (istr("a"), Value::Int(1)));
                assert_eq!(fields[1], (istr("b"), Value::Bool(true)));
            }
            _ => panic!("expected open record"),
        },
        other => panic!("expected record, got {other:?}"),
    }

    let duplicate = ValueExpr::RecordLiteral {
        fields: vec![(istr("a"), int_lit(1)), (istr("a"), null_lit())],
        span: span(),
    };
    let err = eval(&duplicate).expect_err("duplicate keys are rejected");
    assert!(matches!(err, ExecutorError::DataException { .. }));
}
