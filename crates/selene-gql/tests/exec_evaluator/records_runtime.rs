use super::*;

#[test]
fn record_field_access_reads_named_field() {
    // C1: property access on an open `RECORD{...}` value reads the named field
    // (ISO/IEC 39075:2024 clause 20.11 `<property reference>`).
    let score = db_string("score").unwrap();
    let rank = db_string("rank").unwrap();
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
    let score = db_string("score").unwrap();
    let missing = db_string("missing").unwrap();
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
    let inner_key = db_string("inner").unwrap();
    let leaf = db_string("leaf").unwrap();
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
    lit(Literal::String(
        db_string(value).unwrap(),
        span(),
        CharacterStringLiteralKind::Escaped,
    ))
}

fn record_lit(fields: Vec<(&str, ValueExpr)>) -> ValueExpr {
    ValueExpr::RecordLiteral {
        fields: fields
            .into_iter()
            .map(|(name, expr)| (db_string(name).unwrap(), expr))
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
        (db_string("a").unwrap(), GqlType::Integer),
        (db_string("b").unwrap(), GqlType::String),
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
        db_string("inner").unwrap(),
        GqlType::Record(RecordType::Closed(vec![(
            db_string("flag").unwrap(),
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
    let name = db_string("v").unwrap();
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
        key: db_string(key).unwrap(),
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
fn property_exists_on_list_target_is_data_exception() {
    let list = ValueExpr::ListLiteral {
        items: vec![record_lit(vec![("foo", int_lit(1))])],
        span: span(),
    };
    let expr = ValueExpr::PropertyExists {
        target: Box::new(list),
        key: db_string("foo").unwrap(),
        key_source_kind: CharacterStringLiteralKind::Escaped,
        span: span(),
    };

    let err = eval_result(&expr).expect_err("PROPERTY_EXISTS rejects list targets");
    assert!(matches!(err, ExecutorError::DataException { .. }));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}

#[test]
fn property_exists_on_record_target_is_data_exception() {
    let expr = ValueExpr::PropertyExists {
        target: Box::new(record_lit(vec![("foo", int_lit(1))])),
        key: db_string("foo").unwrap(),
        key_source_kind: CharacterStringLiteralKind::Escaped,
        span: span(),
    };

    let err = eval_result(&expr).expect_err("PROPERTY_EXISTS rejects record targets");
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
