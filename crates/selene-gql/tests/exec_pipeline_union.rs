//! Pipeline Union executor tests.

mod exec_common;

use exec_common::{
    ExecFixture, column_values, execute_pattern, execute_read, planned, planned_result, props,
};
use selene_core::{LabelSet, Value};
use selene_gql::{
    AnalyzedType, Binding, BindingTable, BindingTableColumn, BindingTableSchema, ExecutorError,
    GqlType, ImplDefinedCaps, PipelineOp, SetOp, execute_pipeline,
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

fn execute_manual_set_op(
    op: SetOp,
    rhs_source: &str,
    input: BindingTable,
) -> Result<BindingTable, ExecutorError> {
    let rhs = planned(rhs_source);
    let caps = ImplDefinedCaps::default();
    let mut ctx = exec_common::empty_graph_context(&caps);
    execute_pipeline(
        &[PipelineOp::Union {
            op,
            rhs: Box::new(rhs),
        }],
        input,
        &mut ctx,
    )
}

#[test]
fn union_all_concatenates_arms() {
    let table = execute_read("RETURN 1 AS n UNION ALL RETURN 2 AS n");

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn union_dedupes_duplicates_from_either_arm() {
    let table = execute_read("RETURN 1 AS n UNION RETURN 1 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn union_all_keeps_duplicates() {
    let table = execute_read("RETURN 1 AS n UNION ALL RETURN 1 AS n");

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(1)]
    );
}

#[test]
fn three_arm_union_processes_left_to_right() {
    let table = execute_read("RETURN 1 AS n UNION RETURN 1 AS n UNION ALL RETURN 2 AS n");

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn union_of_empty_arms_returns_empty() {
    let table = execute_read("RETURN 1 AS n LIMIT 0 UNION ALL RETURN 2 AS n LIMIT 0");

    assert!(table.is_empty());
    assert_eq!(table.schema().columns.len(), 1);
}

#[test]
fn union_of_populated_and_empty_returns_populated() {
    let table = execute_read("RETURN 1 AS n UNION ALL RETURN 2 AS n LIMIT 0");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn union_of_empty_and_populated_returns_populated() {
    let table = execute_read("RETURN 1 AS n LIMIT 0 UNION ALL RETURN 2 AS n");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(2)]);
}

#[test]
fn three_arm_union_with_middle_limit_is_per_arm_static_attachment() {
    // BRIEF-155 ride-along (#3): `A UNION ALL B LIMIT N UNION ALL C` —
    // grammar.pest binds the LIMIT to arm B's pipeline_statement+ only.
    // Arms A and C run unlimited; arm B is capped at LIMIT N. Result is
    // A_rows ++ first-N-of-B ++ C_rows.
    let table = execute_read(
        "RETURN 1 AS n \
         UNION ALL \
         RETURN 2 AS n LIMIT 0 \
         UNION ALL \
         RETURN 3 AS n",
    );
    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(3)],
        "arm B (LIMIT 0) yields nothing; arms A and C are unaffected"
    );
}

#[test]
fn union_with_matching_arm_column_names_succeeds() {
    // ISO §14.2 SR v: arms that are column name-equal combine cleanly. (Migrated
    // from the prior lenient-relabel fixture, which expected the right arm to be
    // silently relabeled to the left arm's name — now an error, see
    // `union_arms_with_differing_column_names_are_rejected`.)
    let table = execute_read("RETURN 1 AS shared UNION ALL RETURN 2 AS shared");

    assert_eq!(
        table.schema().columns[0]
            .name
            .expect("shared name")
            .as_str(),
        "shared"
    );
    assert_eq!(
        column_values(&table, "shared"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn union_arms_with_differing_column_names_are_rejected() {
    // ISO §14.2 SR v: set-composition arms must be column name-equal. selene
    // binds each arm independently, so this must be caught at lowering, not
    // silently relabeled to the LHS schema.
    let err = planned_result("RETURN 1 AS lhs_name UNION ALL RETURN 2 AS rhs_name")
        .expect_err("mismatched arm names are rejected");

    assert_eq!(err.gqlstatus().as_str(), "42001");
}

#[test]
fn union_with_both_arms_unnamed_succeeds() {
    // Both columns unnamed is still column name-equal (the unnamed↔unnamed
    // bijection), so `RETURN 1 UNION RETURN 2` stays legal.
    let table = execute_read("RETURN 1 UNION ALL RETURN 2");

    assert_eq!(table.row_count(), 2);
}

#[test]
fn union_arms_with_differing_column_counts_returns_data_exception() {
    let err = execute_manual_set_op(
        SetOp::UnionAll,
        "RETURN 2 AS a, 3 AS b",
        int_table("n", &[1]),
    )
    .expect_err("mismatch errors");

    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message == "UNION arms have differing column counts: lhs=1, rhs=2"
    ));
}

#[test]
fn union_of_aggregate_returning_arms_produces_two_rows() {
    let table = execute_read(
        "MATCH (n:Person) RETURN count(*) AS c \
         UNION ALL MATCH (m:Sensor) RETURN count(*) AS c",
    );

    assert_eq!(
        column_values(&table, "c"),
        vec![Value::Int(3), Value::Int(1)]
    );
}

#[test]
fn union_rhs_sees_same_snapshot_as_lhs() {
    let fixture = ExecFixture::build();
    let plan = planned(
        "MATCH (n:Person) RETURN n.name AS name \
         UNION ALL MATCH (n:Person) RETURN n.name AS name",
    );
    let mut ctx = fixture.context_caps(&plan);
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

    let table = execute_pipeline(&plan.pipeline, input, &mut ctx).expect("union executes");
    let names = column_values(&table, "name");

    assert_eq!(names.len(), 6);
    assert!(!names.contains(&Value::String(exec_common::istr("Dina"))));
}

#[test]
fn pattern_union_with_matching_alias_composes() {
    // ISO §14.2 SR v: the arms must be column name-equal. `RETURN *` arms whose
    // bindings differ (`n` vs `m`) are non-combinable; aliasing both to the same
    // name makes them combinable. (Migrated from
    // `pattern_star_union_composes_positionally_with_lhs_schema`, which expected
    // the lenient positional relabel that is now rejected.)
    let table = execute_read(
        "MATCH (n:Person) RETURN n AS node UNION ALL MATCH (m:Sensor) RETURN m AS node",
    );

    assert_eq!(
        table.schema().columns[0]
            .name
            .expect("shared binding")
            .as_str(),
        "node"
    );
    assert_eq!(table.row_count(), 4);
}

#[test]
fn pattern_star_union_with_differing_bindings_is_rejected() {
    // `RETURN *` arms binding `n` vs `m` are not column name-equal (ISO §14.2).
    let err = planned_result("MATCH (n:Person) RETURN * UNION ALL MATCH (m:Sensor) RETURN *")
        .expect_err("differing RETURN * arm bindings are rejected");

    assert_eq!(err.gqlstatus().as_str(), "42001");
}

#[test]
fn intersect_returns_common_rows() {
    let table = execute_manual_set_op(SetOp::Intersect, "RETURN 1 AS n", int_table("n", &[1, 2]))
        .expect("intersect executes");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn except_returns_lhs_only_rows() {
    let table = execute_manual_set_op(SetOp::Except, "RETURN 1 AS n", int_table("n", &[1, 2]))
        .expect("except executes");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(2)]);
}

#[test]
fn otherwise_with_non_empty_lhs_returns_lhs() {
    let table = execute_manual_set_op(SetOp::Otherwise, "RETURN 2 AS n", int_table("n", &[1]))
        .expect("otherwise executes");

    assert_eq!(column_values(&table, "n"), vec![Value::Int(1)]);
}

#[test]
fn union_rhs_pipeline_op_id_high_water_consistent_after_executor_runs() {
    let plan = planned("RETURN 1 AS n LIMIT 1 UNION ALL RETURN 2 AS n LIMIT 1");
    let rhs_high_water = match plan.pipeline.last() {
        Some(PipelineOp::Union { rhs, .. }) => rhs.next_pipeline_op_id.get(),
        other => panic!("expected union op, got {other:?}"),
    };

    let table = execute_read("RETURN 1 AS n LIMIT 1 UNION ALL RETURN 2 AS n LIMIT 1");

    assert_eq!(
        column_values(&table, "n"),
        vec![Value::Int(1), Value::Int(2)]
    );
    assert_eq!(plan.next_pipeline_op_id.get(), plan.pipeline.len() as u32);
    assert_eq!(rhs_high_water, 2);
}
