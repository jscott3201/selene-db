//! Pipeline Filter executor tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, istr, node_ids_for, planned, props};
use selene_core::{LabelSet, Value};
use selene_gql::{ExecutorError, PipelineOp, ValueExpr, execute_pipeline};

#[test]
fn filter_passes_rows_where_expr_is_true() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age > 40 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    let filtered = execute_pipeline(&plan.pipeline[..1], table, &mut ctx).expect("filter executes");

    assert_eq!(node_ids_for(&filtered, "n"), vec![Some(2), Some(3)]);
}

#[test]
fn filter_drops_rows_where_expr_is_false() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age > 100 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    let filtered = execute_pipeline(&plan.pipeline[..1], table, &mut ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn filter_drops_rows_where_expr_is_null() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.missing IS NULL RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.expr = ValueExpr::Literal(selene_gql::Literal::Null(predicate.span));

    let filtered = execute_pipeline(&[filter], table, &mut ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn where_date_a_lt_date_b_filters_rows() {
    let fixture = ExecFixture::build();
    let event = istr("Event");
    let date_a = istr("date_a");
    let date_b = istr("date_b");
    let matching;
    {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        matching = mutator
            .create_node(
                LabelSet::single(event),
                props([
                    (date_a, Value::Date("2024-01-01".parse().unwrap())),
                    (date_b, Value::Date("2024-01-02".parse().unwrap())),
                ]),
            )
            .expect("matching event inserts");
        mutator
            .create_node(
                LabelSet::single(event),
                props([
                    (date_a, Value::Date("2024-01-03".parse().unwrap())),
                    (date_b, Value::Date("2024-01-02".parse().unwrap())),
                ]),
            )
            .expect("non-matching event inserts");
        txn.commit().expect("events commit");
    }

    let plan = planned("MATCH (n:Event) WHERE n.date_a < n.date_b RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let filtered = execute_pipeline(&plan.pipeline, table, &mut ctx).expect("filter executes");

    assert_eq!(node_ids_for(&filtered, "n"), vec![Some(matching.get())]);
}

#[test]
fn filter_drops_rows_where_expr_is_non_bool() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age = 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.expr = ValueExpr::Literal(selene_gql::Literal::String(
        exec_common::istr("not boolean"),
        predicate.span,
    ));

    let filtered = execute_pipeline(&[filter], table, &mut ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn filter_rejects_index_consumed_predicate_in_pipeline() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age = 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let mut ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.index_consumed = true;

    let err = execute_pipeline(&[filter], table, &mut ctx).expect_err("consumed predicate errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "index-consumed predicate emitted into pipeline"
        }
    ));
}
