//! BRIEF-116 evaluator completeness coverage.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::{
    ExecFixture, column_values, db_string, execute_read, execute_read_result, planned, props,
};
use selene_core::{EdgeId, LabelSet, NodeId, Value};
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
    lit(Literal::String(db_string(value), span()))
}

fn null_lit() -> ValueExpr {
    lit(Literal::Null(span()))
}

fn bool_lit(value: bool) -> ValueExpr {
    lit(Literal::Bool(value, span()))
}

fn var(name: selene_core::DbString) -> ValueExpr {
    ValueExpr::Variable { name, span: span() }
}

fn named_column(name: selene_core::DbString) -> BindingTableColumn {
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
    columns: Vec<selene_core::DbString>,
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
    columns: Vec<selene_core::DbString>,
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

fn function_call(name: &str, args: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string(name)]).expect("non-empty"),
        args,
        star: false,
        distinct: false,
        span: span(),
    }
}

fn list_lit(items: Vec<ValueExpr>) -> ValueExpr {
    ValueExpr::ListLiteral {
        items,
        span: span(),
    }
}

fn list_access(target: ValueExpr, index: ValueExpr) -> ValueExpr {
    ValueExpr::ListAccess {
        target: Box::new(target),
        index: Box::new(index),
        span: span(),
    }
}

fn assert_float_near(value: Value, expected: f64) {
    match value {
        Value::Float(actual) => assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        ),
        other => panic!("expected float {expected}, got {other:?}"),
    }
}

fn assert_data_exception_contains(err: ExecutorError, expected: &str) {
    match err {
        ExecutorError::DataException { message, .. } => {
            assert!(
                message.contains(expected),
                "expected data exception containing {expected:?}, got {message:?}"
            );
        }
        other => panic!("expected data exception, got {other:?}"),
    }
}

#[test]
fn scalar_numeric_functions_dispatch() {
    let table = execute_read(
        "RETURN abs(-3) AS abs_value, ceil(1.2) AS ceil_value, floor(1.8) AS floor_value, \
         round(1.6) AS round_value, mod(7, 4) AS mod_value, sqrt(9) AS sqrt_value, \
         power(2, 3) AS power_value, ceiling(1.2) AS ceiling_value",
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
    assert_eq!(
        column_values(&table, "ceiling_value"),
        vec![Value::Float(2.0)]
    );
}

#[test]
fn ceiling_alias_returns_same_as_ceil() {
    assert_eq!(
        single_value("RETURN ceiling(1.2) AS value", "value"),
        single_value("RETURN ceil(1.2) AS value", "value")
    );
}

#[test]
fn scalar_string_and_collection_functions_dispatch() {
    let table = execute_read(
        "RETURN length('abc') AS len, substring('abcdef', 2, 3) AS sub, upper('ab') AS up, \
         lower('AB') AS low, trim(' x ') AS trimmed, coalesce(null, 'x') AS co, \
         size([1, 2, 3]) AS sz, \
         char_length('café') AS char_len, character_length('日本') AS character_len",
    );

    assert_eq!(column_values(&table, "len"), vec![Value::Int(3)]);
    assert_eq!(
        column_values(&table, "sub"),
        vec![Value::String(db_string("cde"))]
    );
    assert_eq!(
        column_values(&table, "up"),
        vec![Value::String(db_string("AB"))]
    );
    assert_eq!(
        column_values(&table, "low"),
        vec![Value::String(db_string("ab"))]
    );
    assert_eq!(
        column_values(&table, "trimmed"),
        vec![Value::String(db_string("x"))]
    );
    assert_eq!(
        column_values(&table, "co"),
        vec![Value::String(db_string("x"))]
    );
    assert_eq!(column_values(&table, "sz"), vec![Value::Int(3)]);
    assert_eq!(column_values(&table, "char_len"), vec![Value::Int(4)]);
    assert_eq!(column_values(&table, "character_len"), vec![Value::Int(2)]);
    assert_eq!(
        eval(&function_call(
            "nullif",
            vec![string_lit("x"), string_lit("x")]
        ))
        .unwrap(),
        Value::Null
    );
}

#[test]
fn substring_null_propagates_index_arguments() {
    assert_eq!(
        single_value("RETURN substring('abc', null, 1) AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN substring('abc', 0, null) AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN substring(null, 0, 1) AS value", "value"),
        Value::Null
    );

    let err = execute_read_result("RETURN substring('abc', -1, 1) AS value")
        .expect_err("negative substring start still errors");
    assert_data_exception_contains(err, "substring start is not a non-negative integer");
}

#[test]
fn scalar_function_errors_have_typed_statuses() {
    let unknown = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string("missing_fn")]).expect("non-empty"),
        args: Vec::new(),
        star: false,
        distinct: false,
        span: span(),
    };
    let err = eval(&unknown).expect_err("unknown function errors");
    assert!(matches!(err, ExecutorError::UnknownFunction { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let bad_arity = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string("abs")]).expect("non-empty"),
        args: Vec::new(),
        star: false,
        distinct: false,
        span: span(),
    };
    let err = eval(&bad_arity).expect_err("wrong arity errors");
    assert!(matches!(err, ExecutorError::FunctionArityMismatch { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let bad_modifier = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![db_string("abs")]).expect("non-empty"),
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
        vec![Value::String(db_string("abcd"))]
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
fn power_negative_integer_exponent_uses_float_path() {
    assert_float_near(single_value("RETURN power(2, -1) AS value", "value"), 0.5);
    assert_float_near(single_value("RETURN power(10, -2) AS value", "value"), 0.01);
    assert_eq!(
        single_value("RETURN power(2, 3) AS value", "value"),
        Value::Int(8)
    );

    let err = execute_read_result("RETURN power(0, -1) AS value")
        .expect_err("zero raised to a negative exponent is invalid");
    assert_eq!(err.gqlstatus().as_str(), "2201F");
    assert_data_exception_contains(err, "power base is zero and exponent is negative");
}

#[test]
fn power_treats_128_bit_integers_as_numeric_for_float_path() {
    let lhs = db_string("lhs");
    let rhs = db_string("rhs");
    let power = function_call("power", vec![var(lhs.clone()), var(rhs.clone())]);

    assert_float_near(
        eval_with_binding(
            &power,
            Binding::new([Value::Int128(2), Value::Int(3)]),
            vec![lhs.clone(), rhs.clone()],
        )
        .expect("Int128 power evaluates"),
        8.0,
    );
    assert_float_near(
        eval_with_binding(
            &power,
            Binding::new([Value::Uint128(10), Value::Int(-2)]),
            vec![lhs, rhs],
        )
        .expect("Uint128 negative power evaluates"),
        0.01,
    );
}

#[test]
fn non_iso_like_and_between_are_rejected_at_parse_time() {
    // `LIKE` and `BETWEEN` are SQL drift; the grammar rejects them so they
    // never reach analysis or execution. ISO replacements (STARTS WITH /
    // ENDS WITH / CONTAINS and `x >= lo AND x <= hi`) cover the same intent
    // and execute at HEAD (asserted below).
    for source in [
        "RETURN 'alphabet' LIKE 'a%bet' AS v",
        "RETURN 5 BETWEEN 1 AND 10 AS v",
    ] {
        let err = selene_gql::parse(source).expect_err(source);
        assert_eq!(
            err.gqlstatus(),
            selene_gql::GqlStatus::SYNTAX_ERROR,
            "{source}",
        );
    }
}

#[test]
fn iso_string_match_and_range_replacements_execute() {
    // STARTS WITH covers the `LIKE 'a%'` prefix intent.
    assert_eq!(
        single_value("RETURN 'alphabet' STARTS WITH 'a' AS v", "v"),
        Value::Bool(true)
    );
    // CONTAINS covers the `LIKE '%bet%'` substring intent.
    assert_eq!(
        single_value("RETURN 'alphabet' CONTAINS 'bet' AS v", "v"),
        Value::Bool(true)
    );
    // `x >= lo AND x <= hi` covers the `BETWEEN lo AND hi` intent.
    assert_eq!(
        single_value("RETURN 5 >= 1 AND 5 <= 10 AS v", "v"),
        Value::Bool(true)
    );
}

#[test]
fn dynamic_reference_ordering_uses_stable_ids() {
    let left = db_string("left");
    let right = db_string("right");
    let lt = ValueExpr::BinaryOp {
        op: BinaryOp::Lt,
        lhs: Box::new(var(left.clone())),
        rhs: Box::new(var(right.clone())),
        span: span(),
    };
    assert_eq!(
        eval_with_binding(
            &lt,
            Binding::new([
                Value::NodeRef(NodeId::new(1)),
                Value::NodeRef(NodeId::new(2))
            ]),
            vec![left.clone(), right.clone()],
        )
        .expect("NodeRef ordering evaluates"),
        Value::Bool(true)
    );

    assert_eq!(
        eval_with_binding(
            &lt,
            Binding::new([
                Value::EdgeRef(EdgeId::new(5)),
                Value::EdgeRef(EdgeId::new(3))
            ]),
            vec![left, right],
        )
        .expect("EdgeRef ordering evaluates"),
        Value::Bool(false)
    );
}

#[test]
fn is_predicates_and_property_exists_use_graph_snapshot() {
    let fixture = ExecFixture::build();
    let node = db_string("node");
    let edge = db_string("edge");

    let labeled = ValueExpr::IsCheck {
        operand: Box::new(var(node.clone())),
        kind: IsCheckKind::Labeled(LabelExpr::Single(fixture.person.clone())),
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &labeled,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            vec![node.clone()],
        )
        .expect("label predicate evaluates"),
        Value::Bool(true)
    );

    let property_exists = ValueExpr::PropertyExists {
        target: Box::new(var(node.clone())),
        key: fixture.name.clone(),
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            vec![node.clone()],
        )
        .expect("property exists evaluates"),
        Value::Bool(true)
    );

    let directed = ValueExpr::IsCheck {
        operand: Box::new(var(edge.clone())),
        kind: IsCheckKind::Directed,
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &directed,
            &fixture,
            Binding::new([Value::EdgeRef(EdgeId::new(1))]),
            vec![edge.clone()],
        )
        .expect("directed predicate evaluates"),
        Value::Bool(true)
    );

    let source = ValueExpr::IsCheck {
        operand: Box::new(var(node.clone())),
        kind: IsCheckKind::SourceOf(Box::new(var(edge.clone()))),
        negated: false,
        span: span(),
    };
    assert_eq!(
        eval_with_fixture(
            &source,
            &fixture,
            Binding::new([
                Value::NodeRef(NodeId::new(1)),
                Value::EdgeRef(EdgeId::new(1))
            ]),
            vec![node, edge],
        )
        .expect("source predicate evaluates"),
        Value::Bool(true)
    );
}

#[test]
fn property_exists_target_null_propagates_but_property_null_is_false() {
    let fixture = ExecFixture::build();
    let node = db_string("node");

    let property_exists = ValueExpr::PropertyExists {
        target: Box::new(var(node.clone())),
        key: fixture.name.clone(),
        span: span(),
    };

    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::Null]),
            vec![node.clone()],
        )
        .expect("null target propagates"),
        Value::Null
    );
    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(1))]),
            vec![node.clone()],
        )
        .expect("present property evaluates"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::NodeRef(NodeId::new(4))]),
            vec![node.clone()],
        )
        .expect("absent property evaluates"),
        Value::Bool(false)
    );

    let null_property_node = {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        let node = mutator
            .create_node(
                LabelSet::single(fixture.sensor.clone()),
                props([(fixture.name.clone(), Value::Null)]),
            )
            .expect("null-property node inserts");
        txn.commit().expect("fixture update commits");
        node
    };
    assert_eq!(
        eval_with_fixture(
            &property_exists,
            &fixture,
            Binding::new([Value::NodeRef(null_property_node)]),
            vec![node],
        )
        .expect("null property evaluates"),
        Value::Bool(false)
    );
}

#[test]
fn is_typed_and_is_normalized_evaluate() {
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
    assert_eq!(
        eval(&normalized).expect("NORMALIZED predicate evaluates"),
        Value::Bool(true)
    );
}

#[test]
fn null_is_typed_matrix_is_two_valued() {
    for (ty, negated, expected) in [
        (GqlType::String, false, false),
        (GqlType::String, true, true),
        (GqlType::Null, false, true),
        (GqlType::Null, true, false),
    ] {
        let expr = ValueExpr::IsCheck {
            operand: Box::new(null_lit()),
            kind: IsCheckKind::Typed(ty),
            negated,
            span: span(),
        };
        assert_eq!(
            eval(&expr).expect("NULL IS TYPED evaluates"),
            Value::Bool(expected)
        );
    }
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
    // 1-based ordinal subscript (ISO §14.8): list[1] is the first element.
    assert_eq!(
        single_value("RETURN [10, 20, 30][1] AS value", "value"),
        Value::Int(10)
    );
    assert_eq!(
        single_value("RETURN [10, 20, 30][2] AS value", "value"),
        Value::Int(20)
    );
    assert_eq!(
        single_value("RETURN [10, 20, 30][3] AS value", "value"),
        Value::Int(30)
    );
    // Ordinal 0 and beyond-cardinality fall outside 1..=cardinality -> NULL.
    assert_eq!(
        single_value("RETURN [10, 20, 30][0] AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN [10, 20, 30][4] AS value", "value"),
        Value::Null
    );
    assert_eq!(
        single_value("RETURN [10][-1] AS value", "value"),
        Value::Null
    );

    assert_eq!(
        eval(&list_access(null_lit(), int_lit(0))).unwrap(),
        Value::Null
    );
    assert_eq!(
        eval(&list_access(
            list_lit(vec![int_lit(1), int_lit(2), int_lit(3)]),
            null_lit()
        ))
        .unwrap(),
        Value::Null
    );
    assert_eq!(
        eval(&list_access(null_lit(), null_lit())).unwrap(),
        Value::Null
    );
    let index = db_string("index");
    assert_eq!(
        eval_with_binding(
            &list_access(
                list_lit(vec![int_lit(1), int_lit(2), int_lit(3)]),
                var(index.clone()),
            ),
            Binding::new([Value::Uint(1)]),
            vec![index.clone()],
        )
        .unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        eval_with_binding(
            &list_access(
                list_lit(vec![int_lit(1), int_lit(2), int_lit(3)]),
                var(index.clone()),
            ),
            Binding::new([Value::Uint(0)]),
            vec![index],
        )
        .unwrap(),
        Value::Null
    );
    assert_eq!(
        eval(&list_access(
            list_lit(vec![int_lit(1), int_lit(2), int_lit(3)]),
            int_lit(-1),
        ))
        .unwrap(),
        Value::Null
    );

    let err = eval(&list_access(string_lit("string"), int_lit(0)))
        .expect_err("non-list target still errors");
    assert_data_exception_contains(err, "list access target is not a list");
    let err = eval(&list_access(
        list_lit(vec![int_lit(1), int_lit(2), int_lit(3)]),
        string_lit("x"),
    ))
    .expect_err("non-integer index still errors");
    assert_data_exception_contains(err, "list access index is not an integer");

    let record = ValueExpr::RecordLiteral {
        fields: vec![
            (db_string("a"), int_lit(1)),
            (db_string("b"), bool_lit(true)),
        ],
        span: span(),
    };
    match eval(&record).expect("record evaluates") {
        Value::Record(record) => match *record {
            selene_core::Record::Open(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0], (db_string("a"), Value::Int(1)));
                assert_eq!(fields[1], (db_string("b"), Value::Bool(true)));
            }
            _ => panic!("expected open record"),
        },
        other => panic!("expected record, got {other:?}"),
    }

    let duplicate = ValueExpr::RecordLiteral {
        fields: vec![(db_string("a"), int_lit(1)), (db_string("a"), null_lit())],
        span: span(),
    };
    let err = eval(&duplicate).expect_err("duplicate keys are rejected");
    assert!(matches!(err, ExecutorError::DataException { .. }));
}
