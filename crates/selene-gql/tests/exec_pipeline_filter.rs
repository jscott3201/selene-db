//! Pipeline Filter executor tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids_for, planned};
use selene_gql::{ExecutorError, PipelineOp, ValueExpr, execute_pipeline};

#[test]
fn filter_passes_rows_where_expr_is_true() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age > 40 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    let filtered = execute_pipeline(&plan.pipeline[..1], table, &ctx).expect("filter executes");

    assert_eq!(node_ids_for(&filtered, "n"), vec![Some(2), Some(3)]);
}

#[test]
fn filter_drops_rows_where_expr_is_false() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age > 100 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);

    let filtered = execute_pipeline(&plan.pipeline[..1], table, &ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn filter_drops_rows_where_expr_is_null() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.missing IS NULL RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.expr = ValueExpr::Literal(selene_gql::Literal::Null(predicate.span));

    let filtered = execute_pipeline(&[filter], table, &ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn filter_drops_rows_where_expr_is_non_bool() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age = 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.expr = ValueExpr::Literal(selene_gql::Literal::String(
        exec_common::istr("not boolean"),
        predicate.span,
    ));

    let filtered = execute_pipeline(&[filter], table, &ctx).expect("filter executes");

    assert!(filtered.is_empty());
}

#[test]
fn filter_rejects_index_consumed_predicate_in_pipeline() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (n:Person) FILTER n.age = 30 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);
    let table = execute_pattern(pattern, &ctx);
    let mut filter = plan.pipeline[0].clone();
    let PipelineOp::Filter(predicate) = &mut filter else {
        panic!("expected filter");
    };
    predicate.index_consumed = true;

    let err = execute_pipeline(&[filter], table, &ctx).expect_err("consumed predicate errors");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "index-consumed predicate emitted into pipeline"
        }
    ));
}
