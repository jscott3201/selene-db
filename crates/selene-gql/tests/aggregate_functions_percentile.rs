//! GF11 PERCENTILE_CONT / PERCENTILE_DISC aggregate coverage.

use std::sync::{Arc, Mutex};

use selene_core::{GraphId, IStr, Value, feature_register::FeatureId, intern};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorWarning, GqlStatus, Session, StatementOutput, WarningSink,
    analyze, feature_walk, parse,
};
use selene_graph::SharedGraph;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn execute_rows(session: &mut Session<'_>, source: &str) -> selene_gql::BindingTable {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("query executes")
    {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute(source: &str) -> selene_gql::BindingTable {
    let graph = SharedGraph::new(GraphId::new(13_500));
    let mut session = Session::new(&graph);
    execute_rows(&mut session, source)
}

fn column_values(table: &selene_gql::BindingTable, name: &str) -> Vec<Value> {
    let index = table
        .schema()
        .columns
        .iter()
        .position(|column| {
            column
                .name
                .clone()
                .is_some_and(|column| column.as_str() == name)
        })
        .expect("column exists");
    table
        .rows()
        .iter()
        .map(|row| row.get(index).cloned().unwrap_or(Value::Null))
        .collect()
}

#[derive(Clone)]
struct RecordingSink {
    warnings: Arc<Mutex<Vec<ExecutorWarning>>>,
}

impl WarningSink for RecordingSink {
    fn emit(&mut self, warning: ExecutorWarning) {
        self.warnings.lock().expect("warning mutex").push(warning);
    }
}

#[test]
fn percentile_cont_uses_linear_interpolation() {
    let odd = execute("UNWIND [1, 2, 3] AS x RETURN percentile_cont(x, 0.5) AS p");
    assert_eq!(column_values(&odd, "p"), vec![Value::Float(2.0)]);

    let even = execute(
        "UNWIND [1, 2, 3, 4] AS x \
         RETURN percentile_cont(x, 0.5) AS mid, \
                percentile_cont(x, 0.0) AS min, \
                percentile_cont(x, 1.0) AS max",
    );
    assert_eq!(column_values(&even, "mid"), vec![Value::Float(2.5)]);
    assert_eq!(column_values(&even, "min"), vec![Value::Float(1.0)]);
    assert_eq!(column_values(&even, "max"), vec![Value::Float(4.0)]);
}

#[test]
fn percentile_disc_uses_ties_even_on_one_based_index() {
    let table = execute(
        "UNWIND [1, 2, 3, 4] AS x \
         RETURN percentile_disc(x, 0.5) AS half, percentile_disc(x, 0.75) AS upper",
    );

    assert_eq!(column_values(&table, "half"), vec![Value::Int(2)]);
    assert_eq!(column_values(&table, "upper"), vec![Value::Int(3)]);
}

#[test]
fn percentile_null_handling_matches_set_function_warning_contract() {
    let graph = SharedGraph::new(GraphId::new(13_501));
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        warnings: Arc::clone(&warnings),
    };
    let mut session = Session::new(&graph).with_warning_sink(sink);

    let empty = execute_rows(
        &mut session,
        "MATCH (n:Missing) RETURN percentile_cont(n.age, 0.5) AS p",
    );
    assert_eq!(column_values(&empty, "p"), vec![Value::Null]);
    assert!(
        warnings.lock().expect("warning mutex").is_empty(),
        "empty groups should not emit null-elimination warnings"
    );

    let all_null = execute_rows(
        &mut session,
        "UNWIND [NULL, NULL] AS x RETURN percentile_cont(x, 0.5) AS p",
    );
    assert_eq!(column_values(&all_null, "p"), vec![Value::Null]);
    let observed = warnings.lock().expect("warning mutex").clone();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0].code,
        GqlStatus::NULL_VALUE_ELIMINATED_IN_SET_FUNCTION
    );
}

#[test]
fn percentile_independent_expression_accepts_parameters_and_arithmetic() {
    let graph = SharedGraph::new(GraphId::new(13_502));
    let mut session = Session::new(&graph);
    session.bind_parameter(istr("p"), Value::Float(0.75));

    let parameterized = execute_rows(
        &mut session,
        "UNWIND [1, 2, 3, 4] AS x RETURN percentile_cont(x, $p) AS p",
    );
    assert_eq!(column_values(&parameterized, "p"), vec![Value::Float(3.25)]);

    let arithmetic = execute_rows(
        &mut session,
        "UNWIND [1, 2, 3, 4] AS x RETURN percentile_cont(x, 0.5 + 0.25) AS p",
    );
    assert_eq!(column_values(&arithmetic, "p"), vec![Value::Float(3.25)]);
}

#[test]
fn percentile_independent_expression_can_reference_group_key() {
    let table = execute(
        "UNWIND [1, 2, 10, 20] AS value \
         LET p = CASE WHEN value < 10 THEN 0.0 ELSE 1.0 END \
         RETURN p, percentile_cont(value, p) AS result GROUP BY p ORDER BY p",
    );

    assert_eq!(
        column_values(&table, "result"),
        vec![Value::Float(1.0), Value::Float(20.0)]
    );
}

#[test]
fn percentile_independent_expression_rejects_per_row_binding_reference() {
    let statement =
        parse("UNWIND [1, 2] AS x RETURN percentile_cont(x, x) AS p").expect("source parses");
    let error = analyze(statement, &EmptyProcedureRegistry, None)
        .expect_err("independent arg cannot reference per-row binding");

    assert_eq!(error.gqlstatus(), GqlStatus::INVALID_REFERENCE);
}

#[test]
fn percentile_independent_expression_rejects_binding_inside_complex_group_key() {
    let statement = parse(
        "UNWIND [1, 2] AS x \
         RETURN x < 3 AS grouped, percentile_cont(x, x) AS p GROUP BY x < 3",
    )
    .expect("source parses");
    let error = analyze(statement, &EmptyProcedureRegistry, None)
        .expect_err("complex group key does not make x a group-key binding");

    assert_eq!(error.gqlstatus(), GqlStatus::INVALID_REFERENCE);
}

#[test]
fn percentile_runtime_errors_use_data_exception_codes() {
    let graph = SharedGraph::new(GraphId::new(13_503));
    let mut session = Session::new(&graph);

    let out_of_range = session
        .execute_source(
            "UNWIND [1] AS x RETURN percentile_cont(x, 2) AS p",
            &EmptyProcedureRegistry,
        )
        .expect_err("percentile outside [0, 1] errors");
    assert_eq!(out_of_range.gqlstatus().as_str(), "22003");

    let invalid_dependent = session
        .execute_source(
            "UNWIND ['x'] AS x RETURN percentile_cont(x, 0.5) AS p",
            &EmptyProcedureRegistry,
        )
        .expect_err("non-numeric dependent value errors");
    assert_eq!(invalid_dependent.gqlstatus().as_str(), "22G03");
}

#[test]
fn percentile_functions_record_gf11_feature() {
    let statement = parse(
        "UNWIND [1, 2, 3] AS x \
         RETURN percentile_cont(x, 0.5) AS c, percentile_disc(x, 0.5) AS d",
    )
    .expect("source parses");
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        observed.contains(&FeatureId::GF11),
        "PERCENTILE aggregate calls should record GF11, observed {observed:?}"
    );
}
