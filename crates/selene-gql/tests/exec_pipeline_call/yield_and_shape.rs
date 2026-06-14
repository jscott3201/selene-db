//! Procedure CALL yield/shape coverage — YIELD projection, cross products,
//! zero/unit-row semantics, and insert-site preservation, split out of the
//! root `exec_pipeline_call` binary to keep it under the repository 700-LOC
//! cap. Reuses the root binary's `TestRegistry` harness via `super::`.

use selene_core::Value;
use selene_gql::{
    AnalyzedType, GqlType, PipelineOp, ProcedureMutability, ProcedureTier, Session, TxContext,
    execute_pipeline, execute_statement,
};

use super::{
    Behavior, column_values, db_string, execute, graph, output, planned, registry_one, rows,
    seed_table,
};

#[test]
fn read_tier_procedure_yields_rows() {
    let registry = registry_one(
        &["pkg", "rows"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(7)]]),
    );

    let table = rows(execute("CALL pkg.rows() YIELD out", &graph(3901), &registry).unwrap());

    assert_eq!(column_values(&table, "out"), vec![Value::Int(7)]);
    assert_eq!(registry.records()[0].tier, ProcedureTier::Graph);
}

#[test]
fn read_tier_procedure_yield_where_filters_projected_rows() {
    let registry = registry_one(
        &["pkg", "scores"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("score", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(7)], vec![Value::Int(10)]]),
    );

    let table = rows(
        execute(
            "CALL pkg.scores() YIELD score WHERE score >= 10",
            &graph(3918),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(column_values(&table, "score"), vec![Value::Int(10)]);
}

#[test]
fn procedure_returning_zero_rows_drops_input_row() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(Vec::new()),
    );
    let plan = planned("CALL pkg.empty() YIELD out", &registry);
    let graph = graph(3902);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).unwrap();

    assert_eq!(table.row_count(), 0);
    assert_eq!(
        table.schema().columns[0].name.clone().unwrap().as_str(),
        "out"
    );
}

#[test]
fn optional_procedure_returning_zero_rows_preserves_input_with_null_yields() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::Integer)],
        Behavior::Return(Vec::new()),
    );

    let table = rows(
        execute(
            "UNWIND [1, 2] AS x OPTIONAL CALL pkg.empty() YIELD out RETURN x, out ORDER BY x",
            &graph(3919),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
    assert_eq!(column_values(&table, "out"), vec![Value::Null, Value::Null]);
    assert_eq!(registry.records().len(), 2);
}

#[test]
fn optional_procedure_without_yields_preserves_input_for_empty_result() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(Vec::new()),
    );

    let table = rows(
        execute(
            "UNWIND [1, 2] AS x OPTIONAL CALL pkg.empty() RETURN x ORDER BY x",
            &graph(3920),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(
        column_values(&table, "x"),
        vec![Value::Int(1), Value::Int(2)]
    );
}

#[test]
fn optional_procedure_yield_schema_relaxes_non_null_columns() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::NotNull(Box::new(GqlType::Integer)))],
        Behavior::Return(Vec::new()),
    );

    let plan = planned("OPTIONAL CALL pkg.empty() YIELD out", &registry);

    assert_eq!(
        plan.output_schema.columns[0].ty,
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn optional_procedure_null_yield_does_not_satisfy_not_null_type_check() {
    let registry = registry_one(
        &["pkg", "empty"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("out", GqlType::NotNull(Box::new(GqlType::Integer)))],
        Behavior::Return(Vec::new()),
    );

    let table = rows(
        execute(
            "OPTIONAL CALL pkg.empty() YIELD out RETURN out IS TYPED INTEGER NOT NULL AS ok",
            &graph(3921),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(column_values(&table, "ok"), vec![Value::Bool(false)]);
}

#[test]
fn procedure_returning_one_unit_row_emits_one_row_per_input() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let plan = planned("CALL pkg.unit()", &registry);
    let graph = graph(3903);
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &registry,
        graph.index_providers(),
    );

    let table = execute_pipeline(&plan.pipeline, seed_table(), &mut ctx).unwrap();

    assert_eq!(table.row_count(), 1);
    assert!(table.schema().columns.is_empty());
    assert!(table.rows()[0].values().is_empty());
}

#[test]
fn read_tier_procedure_cross_products_with_multi_row_input() {
    let registry = registry_one(
        &["pkg", "two"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("y", GqlType::Integer)],
        Behavior::Return(vec![vec![Value::Int(10)], vec![Value::Int(20)]]),
    );

    let table = rows(
        execute(
            "UNWIND [1, 2, 3] AS x CALL pkg.two() YIELD y RETURN x, y",
            &graph(3904),
            &registry,
        )
        .unwrap(),
    );

    assert_eq!(
        column_values(&table, "x"),
        vec![
            Value::Int(1),
            Value::Int(1),
            Value::Int(2),
            Value::Int(2),
            Value::Int(3),
            Value::Int(3)
        ]
    );
    assert_eq!(
        column_values(&table, "y"),
        vec![
            Value::Int(10),
            Value::Int(20),
            Value::Int(10),
            Value::Int(20),
            Value::Int(10),
            Value::Int(20)
        ]
    );
    assert_eq!(registry.records().len(), 3);
}

#[test]
fn read_tier_procedure_yield_named_selects_columns_by_name() {
    let registry = registry_one(
        &["pkg", "values"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![
            output("a", GqlType::Integer),
            output("b", GqlType::Integer),
            output("c", GqlType::Integer),
        ],
        Behavior::Return(vec![vec![Value::Int(1), Value::Int(2), Value::Int(3)]]),
    );

    let table = rows(execute("CALL pkg.values() YIELD c, a", &graph(3905), &registry).unwrap());

    assert_eq!(table.rows()[0].values(), &[Value::Int(3), Value::Int(1)]);
    let names = table
        .schema()
        .columns
        .iter()
        .map(|column| column.name.as_ref().unwrap().as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["c", "a"]);
}

#[test]
fn read_tier_procedure_yield_star_emits_all_columns_in_schema_order() {
    let registry = registry_one(
        &["pkg", "values"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        vec![output("a", GqlType::Integer), output("b", GqlType::String)],
        Behavior::Return(vec![vec![Value::Int(1), Value::String(db_string("two"))]]),
    );

    let table = rows(execute("CALL pkg.values() YIELD *", &graph(3906), &registry).unwrap());

    assert_eq!(
        table.rows()[0].values(),
        &[Value::Int(1), Value::String(db_string("two"))]
    );
}

#[test]
fn call_after_anonymous_insert_preserves_per_row_insert_sites() {
    let registry = registry_one(
        &["pkg", "unit"],
        ProcedureMutability::Read,
        ProcedureTier::Graph,
        Vec::new(),
        Behavior::Return(vec![vec![]]),
    );
    let mut plan = planned("INSERT (:A)-[:E]->(:B) FINISH", &registry);
    let call = planned("CALL pkg.unit()", &registry)
        .pipeline
        .into_iter()
        .next()
        .expect("call op");
    let edge_index = plan
        .pipeline
        .iter()
        .position(|op| {
            matches!(
                op,
                PipelineOp::Mutation(selene_gql::MutationOp::InsertEdge { .. })
            )
        })
        .expect("edge insert op");
    plan.pipeline.insert(edge_index, call);
    let graph = graph(3918);
    let mut session = Session::new(&graph);

    execute_statement(&plan, &mut session, &registry).expect("insert chain executes");

    let snapshot = graph.read();
    assert_eq!(snapshot.node_count(), 2);
    assert_eq!(snapshot.edge_count(), 1);
}
