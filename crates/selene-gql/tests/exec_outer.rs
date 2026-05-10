//! Executor outer join tests.

mod exec_common;

use exec_common::{ExecFixture, execute_pattern, node_ids_for, planned};
use selene_core::Value;

#[test]
fn outer_emits_left_with_right_when_right_has_matches() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Sensor) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2), Some(3)]);
    assert_eq!(node_ids_for(&table, "b"), vec![None, Some(4), None]);
}

#[test]
fn outer_emits_left_with_null_extended_right_when_right_empty() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person) OPTIONAL MATCH (a)-[:MISSING]->(b:Sensor) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2), Some(3)]);
    assert_eq!(node_ids_for(&table, "b"), vec![None, None, None]);
}

#[test]
fn outer_filter_on_optional_binding_keeps_unmatched_left_rows() {
    let fixture = ExecFixture::build();
    let plan = planned(
        "MATCH (a:Person) OPTIONAL MATCH (a)-[:KNOWS]->(b:Sensor) WHERE b.score > 5 RETURN a, b",
    );
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert_eq!(node_ids_for(&table, "a"), vec![Some(1), Some(2), Some(3)]);
    let b_ids = node_ids_for(&table, "b");
    assert_eq!(b_ids.iter().filter(|id| id.is_some()).count(), 1);
    assert_eq!(b_ids.iter().filter(|id| id.is_none()).count(), 2);
}

#[test]
fn outer_null_extension_keeps_schema_type_cell() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Person) OPTIONAL MATCH (a)-[:MISSING]->(b:Sensor) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);
    let b_schema_ty = &table.schema().columns[1].ty;
    let b_binding_ty = &pattern
        .bindings
        .iter()
        .find(|binding| binding.name.as_str() == "b")
        .expect("b binding")
        .ty;

    assert_eq!(b_schema_ty, b_binding_ty);
    assert!(
        table
            .rows()
            .iter()
            .all(|row| matches!(row.get(1), Some(Value::Null)))
    );
}

#[test]
fn outer_handles_left_with_zero_rows() {
    let fixture = ExecFixture::build();
    let plan = planned("MATCH (a:Missing) OPTIONAL MATCH (a)-[:KNOWS]->(b:Sensor) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let ctx = fixture.context_caps(&plan);

    let table = execute_pattern(pattern, &ctx);

    assert!(table.is_empty());
}
