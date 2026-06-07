//! BRIEF-27 CALL planner lowering tests.

use selene_core::DbString;
use selene_gql::{
    AnalyzedStatement, AnalyzedType, EmptyProcedureRegistry, GqlType, PipelineOp, PlannerError,
    ProcedureOutputColumn, ProcedureParameter, YieldKind, analyze, parse, plan,
};
use selene_testing::MockProcedureRegistry;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn registry() -> MockProcedureRegistry {
    MockProcedureRegistry::new()
        .with_procedure(
            vec![db_string("pkg"), db_string("all")],
            Vec::new(),
            vec![
                ProcedureOutputColumn::new(db_string("outA"), GqlType::String),
                ProcedureOutputColumn::new(db_string("outB"), GqlType::Integer),
            ],
        )
        .with_procedure(
            vec![db_string("pkg"), db_string("args")],
            vec![
                ProcedureParameter::new(db_string("a"), GqlType::Integer, false),
                ProcedureParameter::new(db_string("b"), GqlType::String, false),
            ],
            Vec::new(),
        )
}

fn analyzed(source: &str, registry: &MockProcedureRegistry) -> AnalyzedStatement {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry, None).expect("test input analyzes")
}

fn plan_one(source: &str, registry: &MockProcedureRegistry) -> selene_gql::ExecutionPlan {
    let analyzed = analyzed(source, registry);
    plan(&analyzed, registry).expect("test input plans")
}

#[test]
fn top_level_call_without_yield_discards_output_schema() {
    let registry = registry();
    let plan = plan_one("CALL pkg.all()", &registry);
    let [PipelineOp::Call(call)] = plan.pipeline.as_slice() else {
        panic!("expected call op");
    };
    assert!(call.yield_cols.is_empty());
    assert!(plan.output_schema.columns.is_empty());
}

#[test]
fn call_arguments_are_project_exprs() {
    let registry = registry();
    let plan = plan_one("CALL pkg.args(1, 'a')", &registry);
    let [PipelineOp::Call(call)] = plan.pipeline.as_slice() else {
        panic!("expected call op");
    };
    assert_eq!(call.args.len(), 2);
    assert_eq!(call.args[0].ty, AnalyzedType::Resolved(GqlType::Integer));
    assert_eq!(call.args[1].ty, AnalyzedType::Resolved(GqlType::String));
}

#[test]
fn mixed_yield_star_and_alias_matches_analyzer_order() {
    let registry = registry();
    let plan = plan_one("CALL pkg.all() YIELD *, outA AS first", &registry);
    let [PipelineOp::Call(call)] = plan.pipeline.as_slice() else {
        panic!("expected call op");
    };
    assert!(matches!(call.yield_cols[0].column, YieldKind::Star));
    assert!(
        matches!(&call.yield_cols[1].column, YieldKind::Named(name) if name.as_str() == "outA")
    );
    let names = plan
        .output_schema
        .columns
        .iter()
        .map(|column| column.name.as_ref().expect("name").as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["outA", "outB", "first"]);
    assert_eq!(
        plan.output_schema.columns[1].ty,
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn top_level_call_yield_where_lowers_to_filter_after_call() {
    let registry = registry();
    let plan = plan_one("CALL pkg.all() YIELD outB WHERE outB >= 2", &registry);
    let [PipelineOp::Call(call), PipelineOp::Filter(filter)] = plan.pipeline.as_slice() else {
        panic!("expected call followed by filter");
    };

    assert_eq!(call.yield_schema.len(), 1);
    assert_eq!(filter.ty, AnalyzedType::Resolved(GqlType::Boolean));
    assert_eq!(plan.output_schema.columns.len(), 1);
}

#[test]
fn leading_call_can_continue_as_query_pipeline() {
    let registry = registry();
    let plan = plan_one(
        "CALL pkg.all() YIELD outB RETURN outB ORDER BY outB DESC LIMIT 1",
        &registry,
    );
    let [
        PipelineOp::Call(call),
        PipelineOp::Project(project),
        PipelineOp::OrderBy(_),
        PipelineOp::Limit { .. },
    ] = plan.pipeline.as_slice()
    else {
        panic!("expected leading call, project, order, limit pipeline");
    };

    assert_eq!(call.yield_schema.len(), 1);
    assert_eq!(project.len(), 1);
    assert_eq!(plan.output_schema.columns.len(), 1);
    assert_eq!(
        plan.output_schema.columns[0]
            .name
            .as_ref()
            .expect("output is named")
            .as_str(),
        "outB"
    );
}

#[test]
fn in_pipeline_call_extends_visible_columns() {
    let registry = registry();
    let plan = plan_one("MATCH (n) CALL pkg.all() YIELD outA RETURN *", &registry);
    let names = plan
        .output_schema
        .columns
        .iter()
        .filter_map(|column| column.name.as_ref().map(|name| name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(names, ["n", "outA"]);
    assert!(
        plan.pipeline
            .iter()
            .any(|op| matches!(op, PipelineOp::Call(_)))
    );
}

#[test]
fn unknown_procedure_between_analyze_and_plan_is_defensive_error() {
    let registry = registry();
    let analyzed = analyzed("CALL pkg.all() YIELD outA", &registry);
    let err = plan(&analyzed, &EmptyProcedureRegistry).expect_err("registry changed");
    assert!(matches!(
        err,
        PlannerError::UnknownProcedure { procedure, .. }
            if procedure.iter().map(|part| part.as_str()).collect::<Vec<_>>() == ["pkg", "all"]
    ));
}

#[test]
fn yield_star_duplicate_after_registry_drift_is_defensive_error() {
    let registry = registry();
    let analyzed = analyzed("CALL pkg.all() YIELD *, outA AS first", &registry);
    let drifted = MockProcedureRegistry::new().with_procedure(
        vec![db_string("pkg"), db_string("all")],
        Vec::new(),
        vec![
            ProcedureOutputColumn::new(db_string("outA"), GqlType::String),
            ProcedureOutputColumn::new(db_string("outB"), GqlType::Integer),
            ProcedureOutputColumn::new(db_string("first"), GqlType::String),
        ],
    );

    let err = plan(&analyzed, &drifted).expect_err("wildcard collision is defensive error");
    assert!(matches!(
        err,
        PlannerError::ProcedureMetadataMismatch {
            detail: "duplicate yield column after wildcard",
            ..
        }
    ));
}

#[test]
fn sentinel_call_plan_shape_snapshot() {
    let registry = registry();
    let plan = plan_one("CALL pkg.all() YIELD *, outA AS first", &registry);
    let summary = plan
        .output_schema
        .columns
        .iter()
        .map(|column| {
            format!(
                "{}:{:?}",
                column.name.clone().expect("name").as_str(),
                column.ty
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(summary, @r###"
outA:Resolved(String)
outB:Resolved(Integer)
first:Resolved(String)
"###);
}
