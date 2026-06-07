//! Unit coverage for runtime CAST branches that are not addressable
//! through GQL grammar. The integration suite at
//! `crates/selene-gql/tests/cast.rs` covers every grammar-reachable
//! path; these tests close the remaining BF-class gaps (NaN, overflow,
//! ±Infinity) where the grammar has no literal for the input value.

use std::sync::Arc;

use super::*;
use crate::RecordType;
use selene_core::GraphId;
use selene_graph::SeleneGraph;

use crate::{
    EmptyProcedureRegistry, ImplDefinedCaps, SourceSpan, SubqueryRegistry,
    analyze::ExprIdLookup,
    runtime::{ExecutorError, TxContext},
};

fn span() -> SourceSpan {
    SourceSpan::default()
}

fn eval_cast_for_test(
    value: Value,
    target: &GqlType,
    span: SourceSpan,
) -> Result<Value, ExecutorError> {
    let caps = ImplDefinedCaps::default();
    let graph = Arc::new(SeleneGraph::new(GraphId::new(9_411)));
    let tx = TxContext::read_only(graph, &caps, &EmptyProcedureRegistry, &[]);
    let expr_ids = ExprIdLookup::default();
    let subqueries = SubqueryRegistry::default();
    let ctx = EvalCtx {
        tx: &tx,
        expr_ids: &expr_ids,
        subqueries: &subqueries,
    };
    super::eval_cast(value, target, span, &ctx)
}

#[test]
fn float_nan_to_integer_returns_22018() {
    let err = eval_cast_for_test(Value::Float(f64::NAN), &GqlType::Integer, span())
        .expect_err("NaN cast is rejected");
    let ExecutorError::DataException { subclass, .. } = err else {
        panic!("expected DataException, got {err:?}");
    };
    assert_eq!(
        subclass,
        DataExceptionSubclass::InvalidCharacterValueForCast
    );
}

#[test]
fn float_overflow_to_integer_returns_22003() {
    let err = eval_cast_for_test(Value::Float(1e30_f64), &GqlType::Integer, span())
        .expect_err("overflow cast is rejected");
    let ExecutorError::DataException { subclass, .. } = err else {
        panic!("expected DataException, got {err:?}");
    };
    assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
}

#[test]
fn float_negative_overflow_to_integer_returns_22003() {
    let err = eval_cast_for_test(Value::Float(-1e30_f64), &GqlType::Integer, span())
        .expect_err("negative overflow cast is rejected");
    let ExecutorError::DataException { subclass, .. } = err else {
        panic!("expected DataException, got {err:?}");
    };
    assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
}

#[test]
fn float_positive_infinity_to_integer_returns_22003() {
    let err = eval_cast_for_test(Value::Float(f64::INFINITY), &GqlType::Integer, span())
        .expect_err("+inf cast is rejected");
    let ExecutorError::DataException { subclass, .. } = err else {
        panic!("expected DataException, got {err:?}");
    };
    assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
}

#[test]
fn float_negative_infinity_to_integer_returns_22003() {
    let err = eval_cast_for_test(Value::Float(f64::NEG_INFINITY), &GqlType::Integer, span())
        .expect_err("-inf cast is rejected");
    let ExecutorError::DataException { subclass, .. } = err else {
        panic!("expected DataException, got {err:?}");
    };
    assert_eq!(subclass, DataExceptionSubclass::NumericValueOutOfRange);
}

#[test]
fn recordtyped_source_to_closed_record_is_fail_closed() {
    use selene_core::{RecordTypeId, RecordTyped};
    let value = Value::RecordTyped(Box::new(RecordTyped {
        type_id: RecordTypeId::new(1),
        values: [Some(Value::Int(1))].into_iter().collect(),
    }));
    let field = selene_core::db_string("a").expect("db_string field");
    let target = GqlType::Record(RecordType::Closed(vec![(field, GqlType::Integer)]));
    let err = eval_cast_for_test(value, &target, span()).expect_err("RecordTyped source rejected");
    assert!(
        matches!(err, ExecutorError::FeatureNotSupportedYet { .. }),
        "expected FeatureNotSupportedYet, got {err:?}"
    );
    assert_eq!(err.gqlstatus().as_str(), "42N01");
}

use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

fn cast(value: Value, target: &GqlType) -> Value {
    eval_cast_for_test(value, target, span()).expect("cast succeeds")
}

fn cast_status(value: Value, target: &GqlType) -> String {
    eval_cast_for_test(value, target, span())
        .expect_err("cast rejected")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn as_str(value: Value) -> String {
    match value {
        Value::String(db_string) => db_string.as_str().to_owned(),
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn uint_in_range_to_integer() {
    assert_eq!(cast(Value::Uint(7), &GqlType::Integer), Value::Int(7));
}

#[test]
fn uint_above_i64_max_to_integer_returns_22003() {
    assert_eq!(
        cast_status(Value::Uint(u64::MAX), &GqlType::Integer),
        "22003"
    );
}

#[test]
fn int128_in_range_to_integer() {
    assert_eq!(cast(Value::Int128(-9), &GqlType::Integer), Value::Int(-9));
}

#[test]
fn int128_above_i64_max_to_integer_returns_22003() {
    assert_eq!(
        cast_status(Value::Int128(i128::from(i64::MAX) + 1), &GqlType::Integer),
        "22003"
    );
}

#[test]
fn uint128_in_range_to_integer() {
    assert_eq!(cast(Value::Uint128(9), &GqlType::Integer), Value::Int(9));
}

#[test]
fn uint128_above_i64_max_to_integer_returns_22003() {
    assert_eq!(
        cast_status(Value::Uint128(u128::MAX), &GqlType::Integer),
        "22003"
    );
}

#[test]
fn float32_to_integer_truncates() {
    assert_eq!(
        cast(Value::Float32(3.7_f32), &GqlType::Integer),
        Value::Int(3)
    );
}

#[test]
fn uint_to_float() {
    assert_eq!(cast(Value::Uint(42), &GqlType::Float), Value::Float(42.0));
}

#[test]
fn int128_to_float() {
    assert_eq!(
        cast(Value::Int128(-42), &GqlType::Float),
        Value::Float(-42.0)
    );
}

#[test]
fn uint128_to_float() {
    assert_eq!(
        cast(Value::Uint128(42), &GqlType::Float),
        Value::Float(42.0)
    );
}

#[test]
fn float32_to_float() {
    assert_eq!(
        cast(Value::Float32(0.5_f32), &GqlType::Float),
        Value::Float(0.5)
    );
}

#[test]
fn uint_to_string() {
    assert_eq!(as_str(cast(Value::Uint(42), &GqlType::String)), "42");
}

#[test]
fn int128_to_string() {
    assert_eq!(as_str(cast(Value::Int128(-42), &GqlType::String)), "-42");
}

#[test]
fn uint128_to_string() {
    assert_eq!(as_str(cast(Value::Uint128(42), &GqlType::String)), "42");
}

#[test]
fn float32_to_string() {
    assert_eq!(
        as_str(cast(Value::Float32(0.5_f32), &GqlType::String)),
        "0.5"
    );
}

#[test]
fn bool_to_integer_returns_22g03() {
    assert_eq!(cast_status(Value::Bool(true), &GqlType::Integer), "22G03");
}

#[test]
fn bool_to_float_returns_22g03() {
    assert_eq!(cast_status(Value::Bool(true), &GqlType::Float), "22G03");
}

#[test]
fn bool_to_decimal_returns_22g03() {
    assert_eq!(cast_status(Value::Bool(true), &GqlType::Decimal), "22G03");
    assert_eq!(cast_status(Value::Bool(false), &GqlType::Decimal), "22G03");
}

#[test]
fn int_to_boolean_returns_22g03() {
    assert_eq!(cast_status(Value::Int(1), &GqlType::Boolean), "22G03");
    assert_eq!(cast_status(Value::Int(0), &GqlType::Boolean), "22G03");
    assert_eq!(cast_status(Value::Int(2), &GqlType::Boolean), "22G03");
}

#[test]
fn numeric_family_to_boolean_returns_22g03() {
    for value in [
        Value::Uint(1),
        Value::Int128(1),
        Value::Uint128(1),
        Value::Float(1.0),
        Value::Float32(1.0_f32),
        Value::Decimal(Decimal::from_f64(1.0).unwrap()),
    ] {
        assert_eq!(
            cast_status(value.clone(), &GqlType::Boolean),
            "22G03",
            "expected 22G03 for {value:?} -> BOOLEAN"
        );
    }
}

#[test]
fn bool_to_string_is_uppercase() {
    assert_eq!(as_str(cast(Value::Bool(true), &GqlType::String)), "TRUE");
    assert_eq!(as_str(cast(Value::Bool(false), &GqlType::String)), "FALSE");
}

#[test]
fn string_to_boolean_is_case_insensitive() {
    for text in ["true", "True", "TRUE", "tRuE", "  true  "] {
        assert_eq!(
            cast(
                Value::String(selene_core::db_string(text).unwrap()),
                &GqlType::Boolean
            ),
            Value::Bool(true),
            "`{text}` must parse to TRUE"
        );
    }
    for text in ["false", "False", "FALSE", "fAlSe", " FALSE "] {
        assert_eq!(
            cast(
                Value::String(selene_core::db_string(text).unwrap()),
                &GqlType::Boolean
            ),
            Value::Bool(false),
            "`{text}` must parse to FALSE"
        );
    }
}

#[test]
fn string_to_boolean_garbage_still_returns_22018() {
    assert_eq!(
        cast_status(
            Value::String(selene_core::db_string("yes").unwrap()),
            &GqlType::Boolean
        ),
        "22018"
    );
}
