//! Executor expression evaluator tests.

#![cfg(feature = "test-harness")]

mod exec_common;

use exec_common::empty_graph_context;
use selene_core::{Value, intern};
use selene_gql::{
    AnalyzedType, BinaryOp, Binding, BindingTableColumn, BindingTableSchema, ExecutorError,
    GqlType, ImplDefinedCaps, IsCheckKind, Literal, NonEmpty, RecordType, SourceSpan, TruthValue,
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
    assert_eq!(err.gqlstatus().as_str(), "22003");
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
    assert_eq!(err.gqlstatus().as_str(), "22003");
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
    assert_eq!(err.gqlstatus().as_str(), "22G04");
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

#[test]
fn record_field_access_reads_named_field() {
    // C1: property access on an open `RECORD{...}` value reads the named field
    // (ISO/IEC 39075:2024 clause 20.11 `<property reference>`).
    let score = intern("score").unwrap();
    let rank = intern("rank").unwrap();
    let record = ValueExpr::RecordLiteral {
        fields: vec![(score.clone(), int_lit(7)), (rank, int_lit(2))],
        span: span(),
    };
    let access = ValueExpr::PropertyAccess {
        target: Box::new(record),
        key: score,
        span: span(),
    };

    assert_eq!(eval(&access), Value::Int(7));
}

#[test]
fn record_field_access_absent_field_is_null() {
    // An open record yields NULL for a field it does not carry (open-record
    // property-reference declared type is the nullable open dynamic union type).
    let score = intern("score").unwrap();
    let missing = intern("missing").unwrap();
    let record = ValueExpr::RecordLiteral {
        fields: vec![(score, int_lit(7))],
        span: span(),
    };
    let access = ValueExpr::PropertyAccess {
        target: Box::new(record),
        key: missing,
        span: span(),
    };

    assert_eq!(eval(&access), Value::Null);
}

#[test]
fn nested_record_field_access_reads_inner_field() {
    // Field access composes: the outer field resolves to a record value, and a
    // second property access reads a field from that inner record.
    let inner_key = intern("inner").unwrap();
    let leaf = intern("leaf").unwrap();
    let inner = ValueExpr::RecordLiteral {
        fields: vec![(leaf.clone(), int_lit(42))],
        span: span(),
    };
    let outer = ValueExpr::RecordLiteral {
        fields: vec![(inner_key.clone(), inner)],
        span: span(),
    };
    let access = ValueExpr::PropertyAccess {
        target: Box::new(ValueExpr::PropertyAccess {
            target: Box::new(outer),
            key: inner_key,
            span: span(),
        }),
        key: leaf,
        span: span(),
    };

    assert_eq!(eval(&access), Value::Int(42));
}

// --- IS [NOT] TYPED RECORD / LIST structural runtime (Group J) ---

fn string_lit(value: &str) -> ValueExpr {
    lit(Literal::String(intern(value).unwrap(), span()))
}

fn record_lit(fields: Vec<(&str, ValueExpr)>) -> ValueExpr {
    ValueExpr::RecordLiteral {
        fields: fields
            .into_iter()
            .map(|(name, expr)| (intern(name).unwrap(), expr))
            .collect(),
        span: span(),
    }
}

fn is_typed(operand: ValueExpr, ty: GqlType, negated: bool) -> ValueExpr {
    ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: IsCheckKind::Typed(ty),
        negated,
        span: span(),
    }
}

fn closed_ab() -> GqlType {
    GqlType::Record(RecordType::Closed(vec![
        (intern("a").unwrap(), GqlType::Integer),
        (intern("b").unwrap(), GqlType::String),
    ]))
}

#[test]
fn is_typed_closed_record_is_structural() {
    let conforming = || record_lit(vec![("a", int_lit(1)), ("b", string_lit("x"))]);
    // Conforming closed record → true; IS NOT TYPED negates to false.
    assert_eq!(
        eval(&is_typed(conforming(), closed_ab(), false)),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&is_typed(conforming(), closed_ab(), true)),
        Value::Bool(false)
    );
    // Extra undeclared field → false (ISO §4.15.4 field-name-set equality).
    assert_eq!(
        eval(&is_typed(
            record_lit(vec![
                ("a", int_lit(1)),
                ("b", string_lit("x")),
                ("c", int_lit(2))
            ]),
            closed_ab(),
            false,
        )),
        Value::Bool(false)
    );
    // Missing declared field → false.
    assert_eq!(
        eval(&is_typed(
            record_lit(vec![("a", int_lit(1))]),
            closed_ab(),
            false
        )),
        Value::Bool(false)
    );
    // Wrong field type → false.
    assert_eq!(
        eval(&is_typed(
            record_lit(vec![("a", string_lit("x")), ("b", string_lit("y"))]),
            closed_ab(),
            false,
        )),
        Value::Bool(false)
    );
}

#[test]
fn is_typed_nested_and_open_record() {
    // Nested closed record (GV48): RECORD{ inner :: RECORD{ flag :: BOOL } }.
    let ty = GqlType::Record(RecordType::Closed(vec![(
        intern("inner").unwrap(),
        GqlType::Record(RecordType::Closed(vec![(
            intern("flag").unwrap(),
            GqlType::Boolean,
        )])),
    )]));
    let conforming = record_lit(vec![("inner", record_lit(vec![("flag", bool_lit(true))]))]);
    assert_eq!(
        eval(&is_typed(conforming, ty.clone(), false)),
        Value::Bool(true)
    );
    let bad = record_lit(vec![("inner", record_lit(vec![("flag", int_lit(0))]))]);
    assert_eq!(eval(&is_typed(bad, ty, false)), Value::Bool(false));

    // Open record type accepts any record; rejects a non-record.
    let open = || GqlType::Record(RecordType::Open);
    assert_eq!(
        eval(&is_typed(
            record_lit(vec![("x", int_lit(1))]),
            open(),
            false
        )),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&is_typed(int_lit(5), open(), false)),
        Value::Bool(false)
    );
}

#[test]
fn is_typed_list_enforces_element_type() {
    // Regression: the prior runtime ignored the LIST element type, so a list of ints
    // wrongly satisfied LIST<STRING>. It must now be false.
    let list = ValueExpr::ListLiteral {
        items: vec![int_lit(1), int_lit(2)],
        span: span(),
    };
    assert_eq!(
        eval(&is_typed(
            list.clone(),
            GqlType::List(Box::new(GqlType::String)),
            false
        )),
        Value::Bool(false)
    );
    // A list of ints does satisfy LIST<INTEGER>.
    assert_eq!(
        eval(&is_typed(
            list,
            GqlType::List(Box::new(GqlType::Integer)),
            false
        )),
        Value::Bool(true)
    );
}

/// Evaluate `- <value>` by binding the operand to a variable (so non-literal
/// numeric `Value` variants can be exercised) and negating it.
fn negate_value(value: Value) -> Result<Value, ExecutorError> {
    let name = intern("v").unwrap();
    let expr = ValueExpr::UnaryOp {
        op: UnaryOp::Negate,
        operand: Box::new(var(name.clone())),
        span: span(),
    };
    let binding = Binding::new([value]);
    let schema = BindingTableSchema {
        columns: vec![named_column(name)],
    };
    eval_with_binding(&expr, &binding, &schema)
}

// GQLRT-01: unary negate must accept every numeric `Value` variant, not just
// `Int`/`Float`. The analyzer types `-$p` as Dynamic and passes it through, so
// the runtime is the only line of defense; rejecting a numeric operand here is
// a wrong-answer/spurious-error bug.
#[test]
fn negate_over_each_numeric_value_type() {
    // i64
    assert_eq!(negate_value(Value::Int(5)).unwrap(), Value::Int(-5));
    // i64 overflow boundary: -i64::MIN is not representable.
    let err = negate_value(Value::Int(i64::MIN)).expect_err("i64::MIN negate overflows");
    assert_eq!(err.gqlstatus().as_str(), "22003");

    // f64
    assert_eq!(negate_value(Value::Float(2.5)).unwrap(), Value::Float(-2.5));

    // u64 promotes to i64.
    assert_eq!(negate_value(Value::Uint(5)).unwrap(), Value::Int(-5));
    // u64 that does not fit i64 → NumericValueOutOfRange.
    let err = negate_value(Value::Uint(u64::MAX)).expect_err("u64::MAX negate exceeds i64 range");
    assert_eq!(err.gqlstatus().as_str(), "22003");
    // u64 at exactly i64::MAX still fits (boundary accept).
    assert_eq!(
        negate_value(Value::Uint(i64::MAX as u64)).unwrap(),
        Value::Int(-i64::MAX)
    );

    // i128
    assert_eq!(negate_value(Value::Int128(7)).unwrap(), Value::Int128(-7));
    let err = negate_value(Value::Int128(i128::MIN)).expect_err("i128::MIN negate overflows");
    assert_eq!(err.gqlstatus().as_str(), "22003");

    // u128 promotes to i128.
    assert_eq!(negate_value(Value::Uint128(7)).unwrap(), Value::Int128(-7));
    let err =
        negate_value(Value::Uint128(u128::MAX)).expect_err("u128::MAX negate exceeds i128 range");
    assert_eq!(err.gqlstatus().as_str(), "22003");
    // u128 at exactly i128::MAX still fits (boundary accept).
    assert_eq!(
        negate_value(Value::Uint128(i128::MAX as u128)).unwrap(),
        Value::Int128(-i128::MAX)
    );

    // f32 negates within f32.
    assert_eq!(
        negate_value(Value::Float32(1.5)).unwrap(),
        Value::Float32(-1.5)
    );

    // Decimal has a unary negation.
    assert_eq!(
        negate_value(Value::Decimal("3.25".parse().unwrap())).unwrap(),
        Value::Decimal("-3.25".parse().unwrap())
    );

    // Null propagates.
    assert_eq!(negate_value(Value::Null).unwrap(), Value::Null);

    // A non-numeric operand still errors.
    let err = negate_value(Value::Bool(true)).expect_err("boolean negate is a data exception");
    assert!(matches!(err, ExecutorError::DataException { .. }));
}

// --- GQLRT-20: property access on a non-element / non-record target ---

fn property_access_expr(target: ValueExpr, key: &str) -> ValueExpr {
    ValueExpr::PropertyAccess {
        target: Box::new(target),
        key: intern(key).unwrap(),
        span: span(),
    }
}

#[test]
fn property_access_on_list_target_is_data_exception() {
    // `[1,2,3].foo` types the target as Dynamic at analysis, so the type error
    // surfaces at runtime — as 22G03 (a data exception), not 5GQL0.
    let list = ValueExpr::ListLiteral {
        items: vec![int_lit(1), int_lit(2), int_lit(3)],
        span: span(),
    };
    let err = eval_result(&property_access_expr(list, "foo"))
        .expect_err("property access on a list errors");
    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn property_access_on_integer_target_is_data_exception() {
    // `(123).foo` — a scalar target is likewise a 22G03 runtime type error.
    let err = eval_result(&property_access_expr(int_lit(123), "foo"))
        .expect_err("property access on an integer errors");
    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

// --- GQLRT-21: IS <truth value> on a non-boolean operand ---

fn is_truth_value(operand: ValueExpr, truth_value: TruthValue) -> ValueExpr {
    ValueExpr::IsCheck {
        operand: Box::new(operand),
        kind: IsCheckKind::TruthValue(truth_value),
        negated: false,
        span: span(),
    }
}

#[test]
fn is_truth_value_on_boolean_operand_evaluates() {
    assert_eq!(
        eval(&is_truth_value(bool_lit(true), TruthValue::True)),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&is_truth_value(bool_lit(true), TruthValue::False)),
        Value::Bool(false)
    );
    // NULL maps to UNKNOWN per §19 three-valued logic.
    assert_eq!(
        eval(&is_truth_value(null_lit(), TruthValue::Unknown)),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&is_truth_value(null_lit(), TruthValue::True)),
        Value::Bool(false)
    );
}

#[test]
fn is_truth_value_on_non_boolean_operand_is_data_exception() {
    // `5 IS TRUE` is a type error (22G03), not a silent FALSE (the prior dead
    // `_ => false` arm).
    let err = eval_result(&is_truth_value(int_lit(5), TruthValue::True))
        .expect_err("non-boolean IS TRUE errors");
    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");

    let err = eval_result(&is_truth_value(string_lit("x"), TruthValue::False))
        .expect_err("non-boolean IS FALSE errors");
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}
