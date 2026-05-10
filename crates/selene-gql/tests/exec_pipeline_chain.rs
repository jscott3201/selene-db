//! Pipeline Chain executor tests.

mod exec_common;

use exec_common::{ExecFixture, column_values, execute_pattern, execute_read, planned, props};
use selene_core::{LabelSet, Value};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, GqlType,
    ImplDefinedCaps, PipelineOp, execute_pipeline,
};

fn int_table(name: &str, values: &[i64]) -> BindingTable {
    BindingTable::new(
        BindingTableSchema {
            columns: vec![BindingTableColumn {
                name: Some(exec_common::istr(name)),
                hidden: None,
                ty: AnalyzedType::Resolved(GqlType::Integer),
            }],
        },
        values
            .iter()
            .map(|value| Binding::new([Value::Int(*value)]))
            .collect(),
    )
}

#[test]
fn chain_replaces_schema_completely_does_not_extend_lhs_columns() {
    let table = execute_read("RETURN 1 AS a NEXT RETURN 2 AS b");

    let names = table
        .schema()
        .columns
        .iter()
        .filter_map(|column| column.name.map(|name| name.as_str().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["b".to_owned()]);
    assert_eq!(column_values(&table, "b"), vec![Value::Int(2)]);
}

#[test]
fn chain_discards_lhs_rows_when_rhs_has_no_input_refs() {
    let table = execute_read("UNWIND [1, 2, 3] AS a RETURN a NEXT RETURN 9 AS b");

    assert_eq!(column_values(&table, "b"), vec![Value::Int(9)]);
}

#[test]
fn correlated_next_returns_planner_not_implemented() {
    use selene_gql::{EmptyProcedureRegistry, PlannerError, analyze, parse, plan};

    let parsed = parse("UNWIND [1, 2] AS a RETURN a NEXT RETURN a + 10 AS b").expect("parses");
    let analyzed = analyze(parsed, &EmptyProcedureRegistry, None).expect("analyzes");
    let err = plan(&analyzed, &EmptyProcedureRegistry).expect_err("correlated NEXT rejected");

    assert!(matches!(
        err,
        PlannerError::NotImplemented {
            feature: "correlated NEXT (RHS references prior-block bindings)",
            ..
        }
    ));
}

#[test]
fn chain_rhs_can_be_composite_union_plan() {
    let rhs = planned("RETURN 1 AS b UNION ALL RETURN 2 AS b");
    let caps = ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);

    let table = execute_pipeline(
        &[PipelineOp::Chain(Box::new(rhs))],
        int_table("a", &[99]),
        &ctx,
    )
    .expect("chain executes");

    assert_eq!(
        column_values(&table, "b"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn chain_with_empty_rhs_returns_empty() {
    let table = execute_read("RETURN 1 AS a NEXT RETURN 2 AS b LIMIT 0");

    assert!(table.is_empty());
    assert_eq!(
        table.schema().columns[0].name.expect("rhs name").as_str(),
        "b"
    );
}

#[test]
fn nested_chain_blocks_each_replace_schema() {
    let table = execute_read("RETURN 1 AS a NEXT RETURN 2 AS b NEXT RETURN 3 AS c");

    assert_eq!(
        table.schema().columns[0].name.expect("final name").as_str(),
        "c"
    );
    assert_eq!(column_values(&table, "c"), vec![Value::Int(3)]);
}

#[test]
fn chain_rhs_sees_same_snapshot_as_lhs() {
    let fixture = ExecFixture::build();
    // Use distinct binding names in each block so the rhs is uncorrelated;
    // reusing `n` across NEXT triggers the analyzer's boundary=false binding
    // reuse and would (correctly) be rejected as correlated NEXT by the
    // planner's BRIEF-35 §O guard.
    let plan = planned(
        "MATCH (n:Person) RETURN n.name AS name \
         NEXT MATCH (m:Person) RETURN m.name AS name",
    );
    let ctx = fixture.context_caps(&plan);
    {
        let mut txn = fixture.graph.begin_write();
        let mut mutator = txn.mutator();
        mutator
            .create_node(
                LabelSet::single(fixture.person),
                props([(fixture.name, Value::String(exec_common::istr("Dina")))]),
            )
            .expect("late node inserts");
        txn.commit().expect("late write commits");
    }
    let input = execute_pattern(plan.pattern_plan.as_ref().expect("lhs pattern"), &ctx);

    let table = execute_pipeline(&plan.pipeline, input, &ctx).expect("chain executes");
    let names = column_values(&table, "name");

    assert_eq!(names.len(), 3);
    assert!(!names.contains(&Value::String(exec_common::istr("Dina"))));
}

#[test]
fn chain_rhs_order_by_limit_uses_rhs_pipeline() {
    let table = execute_read(
        "UNWIND [3, 1, 2] AS a RETURN a ORDER BY a LIMIT 1 \
         NEXT UNWIND [2, 1] AS b RETURN b ORDER BY b DESC LIMIT 1",
    );

    assert_eq!(column_values(&table, "b"), vec![Value::Int(2)]);
}
