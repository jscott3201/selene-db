//! Pipeline aggregate executor tests.

mod exec_common;

use selene_core::Value;

use exec_common::{column_values, execute_read};

#[test]
fn avg_empty_group_returns_null_not_division_by_zero() {
    let table = execute_read("MATCH (n:Missing) RETURN avg(n.age) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Null]);
}

#[test]
fn avg_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN avg(x) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Float(2.0)]);
}

#[test]
fn avg_preserves_integer_precision_when_inputs_are_int() {
    let table = execute_read("UNWIND [2, 4] AS x RETURN avg(x) AS a");

    assert_eq!(column_values(&table, "a"), vec![Value::Float(3.0)]);
}

#[test]
fn stddev_pop_and_samp_use_welford_accumulator() {
    let table = execute_read(
        "UNWIND [2, 4, 4, 4, 5, 5, 7, 9] AS x \
         RETURN stddev_pop(x) AS pop, stddev_samp(x) AS samp",
    );

    assert_eq!(column_values(&table, "pop"), vec![Value::Float(2.0)]);
    let values = column_values(&table, "samp");
    let [Value::Float(sample)] = values.as_slice() else {
        panic!("expected sample stddev float, got {values:?}");
    };
    assert!((sample - 2.138_089_935_299_395).abs() < 1.0e-12);
}

#[test]
fn stddev_is_numerically_stable_under_large_offset() {
    // GQLRT-25: the existing stddev test uses small magnitudes [2,4,..,9], where
    // a naive single-pass Σx² accumulator gives the right answer within
    // tolerance. Shifting the same data by a ~1e9 offset leaves the true
    // standard deviation unchanged (variance is translation-invariant) but makes
    // a naive Σx² variance catastrophically cancel: each (1e9 + k)^2 loses the
    // low-order bits, so sumsq/n - mean^2 collapses to ~0. Welford's online
    // algorithm stays accurate. This data would FAIL (return ~0) under a
    // naive-variance regression.
    let offset: i64 = 1_000_000_000;
    let values = [2i64, 4, 4, 4, 5, 5, 7, 9]
        .iter()
        .map(|value| (value + offset).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let table = execute_read(&format!(
        "UNWIND [{values}] AS x RETURN stddev_pop(x) AS pop, stddev_samp(x) AS samp"
    ));

    let pop_values = column_values(&table, "pop");
    let [Value::Float(pop)] = pop_values.as_slice() else {
        panic!("expected pop stddev float, got {pop_values:?}");
    };
    // The true population stddev of the unshifted data is exactly 2.0; Welford
    // recovers it to within f64 round-off even at the 1e9 offset. A naive Σx²
    // accumulator returns ~0 here, which the `> 1.0` bound also excludes.
    assert!(
        (pop - 2.0).abs() < 1.0e-6,
        "Welford stddev_pop must stay ~2.0 under a 1e9 offset, got {pop}"
    );
    assert!(
        *pop > 1.0,
        "a naive Σx² variance would cancel to ~0 here; {pop} proves it did not"
    );

    let samp_values = column_values(&table, "samp");
    let [Value::Float(samp)] = samp_values.as_slice() else {
        panic!("expected samp stddev float, got {samp_values:?}");
    };
    assert!(
        (samp - 2.138_089_935_299_395).abs() < 1.0e-6,
        "Welford stddev_samp must match the unshifted value, got {samp}"
    );
}

#[test]
fn stddev_skips_null_inputs_and_sample_requires_two_values() {
    let table = execute_read(
        "UNWIND [1, NULL, 3] AS x \
         RETURN stddev_pop(x) AS pop, stddev_samp(NULL) AS samp",
    );

    assert_eq!(column_values(&table, "pop"), vec![Value::Float(1.0)]);
    assert_eq!(column_values(&table, "samp"), vec![Value::Null]);
}

#[test]
fn multiple_sum_aggregates_with_distinct_arguments_disambiguate() {
    let table =
        execute_read("MATCH (n:Person) RETURN sum(n.age) AS age_sum, sum(n.score) AS score_sum");

    assert_eq!(column_values(&table, "age_sum"), vec![Value::Int(127)]);
    assert_eq!(column_values(&table, "score_sum"), vec![Value::Int(19)]);
}

#[test]
fn unaliased_duplicate_aggregate_functions_disambiguate_by_position() {
    let table = execute_read("MATCH (n:Person) RETURN sum(n.age), sum(n.score)");

    assert_eq!(table.rows().len(), 1);
    assert_eq!(table.rows()[0].values(), &[Value::Int(127), Value::Int(19)]);
}

#[test]
fn having_unaliased_aggregate_disambiguates_from_returned_aggregate() {
    let table =
        execute_read("MATCH (n:Person) RETURN sum(n.age) AS age_sum HAVING sum(n.score) < 20");

    assert_eq!(column_values(&table, "age_sum"), vec![Value::Int(127)]);
}

#[test]
fn sum_empty_returns_int_zero_not_null() {
    let table = execute_read("MATCH (n:Missing) RETURN sum(n.age) AS s");

    assert_eq!(column_values(&table, "s"), vec![Value::Int(0)]);
}

#[test]
fn sum_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN sum(x) AS s");

    assert_eq!(column_values(&table, "s"), vec![Value::Int(4)]);
}

#[test]
fn sum_widens_past_i64_into_i128() {
    // GQLRT-27: an i64 SUM that overflows i64 widens to i128 rather than
    // erroring (GV13/GV14 128-bit support is honest). `i64::MAX + 1` is exactly
    // `i128::from(i64::MAX) + 1`.
    let table = execute_read("UNWIND [9223372036854775807, 1] AS x RETURN sum(x) AS s");

    assert_eq!(
        column_values(&table, "s"),
        vec![Value::Int128(i128::from(i64::MAX) + 1)]
    );
}

#[test]
fn count_skips_null_inputs() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN count(x) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(2)]);
}

#[test]
fn count_empty_returns_zero() {
    let table = execute_read("MATCH (n:Missing) RETURN count(n.age) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(0)]);
}

#[test]
fn count_star_includes_null_rows() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN count(*) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(3)]);
}

#[test]
fn count_distinct_dedups_cross_type_numeric_equivalents() {
    let table = execute_read("UNWIND [1, 1.0, 2, 2.0] AS x RETURN count(DISTINCT x) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(2)]);
}

#[test]
fn aggregate_distinct_uses_runtime_eq_semantics() {
    let table = execute_read("UNWIND [1, 1.0, 1] AS x RETURN count(DISTINCT x) AS c");

    assert_eq!(column_values(&table, "c"), vec![Value::Int(1)]);
}

#[test]
fn min_max_empty_returns_null() {
    let table = execute_read("MATCH (n:Missing) RETURN min(n.age) AS mn, max(n.age) AS mx");

    assert_eq!(column_values(&table, "mn"), vec![Value::Null]);
    assert_eq!(column_values(&table, "mx"), vec![Value::Null]);
}

#[test]
fn min_max_skip_null_inputs() {
    let table = execute_read("UNWIND [2, NULL, 4, 1] AS x RETURN min(x) AS mn, max(x) AS mx");

    assert_eq!(column_values(&table, "mn"), vec![Value::Int(1)]);
    assert_eq!(column_values(&table, "mx"), vec![Value::Int(4)]);
}

#[test]
fn collect_preserves_input_order() {
    let table = execute_read("UNWIND [3, 1, 2] AS x RETURN collect(x) AS xs");

    assert_eq!(
        column_values(&table, "xs"),
        vec![Value::List(vec![
            Value::Int(3),
            Value::Int(1),
            Value::Int(2)
        ])]
    );
}

#[test]
fn collect_includes_nulls() {
    let table = execute_read("UNWIND [1, NULL, 3] AS x RETURN collect(x) AS xs");

    assert_eq!(
        column_values(&table, "xs"),
        vec![Value::List(vec![Value::Int(1), Value::Null, Value::Int(3)])]
    );
}

#[test]
fn collect_distinct_dedups_cross_type_numeric_equivalents() {
    let table = execute_read("UNWIND [1, 1.0, 2] AS x RETURN collect(DISTINCT x) AS xs");
    let values = column_values(&table, "xs");

    assert_eq!(values.len(), 1);
    let Value::List(items) = &values[0] else {
        panic!("expected list, got {:?}", values[0]);
    };
    assert_eq!(items, &vec![Value::Int(1), Value::Int(2)]);
}

#[test]
fn collect_empty_returns_empty_list() {
    let table = execute_read("MATCH (n:Missing) RETURN collect(n.age) AS xs");

    assert_eq!(column_values(&table, "xs"), vec![Value::List(Vec::new())]);
}

#[cfg(feature = "test-harness")]
#[test]
fn function_call_with_let_shadow_does_not_misread_column() {
    use selene_gql::{
        AnalyzedType, Binding, BindingTableColumn, BindingTableSchema, ExecutorError,
        ImplDefinedCaps, NonEmpty, SourceSpan, ValueExpr, runtime::evaluate_for_test,
    };

    let sum = exec_common::istr("sum");
    let x = exec_common::istr("x");
    let schema = BindingTableSchema {
        columns: vec![
            BindingTableColumn {
                name: Some(sum.clone()),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
            BindingTableColumn {
                name: Some(x.clone()),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
        ],
    };
    let row = Binding::new(vec![Value::Int(5), Value::Int(1)]);
    let caps = ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);
    let expr = ValueExpr::FunctionCall {
        name: NonEmpty::try_from_vec(vec![sum]).expect("non-empty"),
        args: vec![ValueExpr::Variable {
            name: x,
            span: SourceSpan::new(4, 1),
        }],
        star: false,
        distinct: false,
        span: SourceSpan::new(0, 6),
    };
    let err = evaluate_for_test(&expr, &row, &schema, &ctx)
        .expect_err("function call should not resolve to same-named column");

    assert!(matches!(
        &err,
        ExecutorError::UnknownFunction { name, .. } if name == "sum"
    ));
    assert_eq!(err.gqlstatus().as_str(), "22G03");
}
