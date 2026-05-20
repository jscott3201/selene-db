//! BRIEF-27 CALL planner lowering tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalyzedStatement, AnalyzedType, EmptyProcedureRegistry, GqlType, PipelineOp, PlannerError,
    ProcedureOutputColumn, ProcedureParameter, YieldKind, analyze, parse, plan,
};
use selene_testing::MockProcedureRegistry;

fn istr(value: &str) -> IStr {
    intern(value).expect("test interner")
}

fn registry() -> MockProcedureRegistry {
    MockProcedureRegistry::new()
        .with_procedure(
            vec![istr("pkg"), istr("all")],
            Vec::new(),
            vec![
                ProcedureOutputColumn::new(istr("outA"), GqlType::String),
                ProcedureOutputColumn::new(istr("outB"), GqlType::Integer),
            ],
        )
        .with_procedure(
            vec![istr("pkg"), istr("args")],
            vec![
                ProcedureParameter::new(istr("a"), GqlType::Integer, false),
                ProcedureParameter::new(istr("b"), GqlType::String, false),
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
    assert!(matches!(call.yield_cols[1].column, YieldKind::Named(name) if name.as_str() == "outA"));
    let names = plan
        .output_schema
        .columns
        .iter()
        .map(|column| column.name.expect("name").as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["outA", "outB", "first"]);
    assert_eq!(
        plan.output_schema.columns[1].ty,
        AnalyzedType::Resolved(GqlType::Integer)
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
        .filter_map(|column| column.name.map(|name| name.as_str()))
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
        vec![istr("pkg"), istr("all")],
        Vec::new(),
        vec![
            ProcedureOutputColumn::new(istr("outA"), GqlType::String),
            ProcedureOutputColumn::new(istr("outB"), GqlType::Integer),
            ProcedureOutputColumn::new(istr("first"), GqlType::String),
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
        .map(|column| format!("{}:{:?}", column.name.expect("name").as_str(), column.ty))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(summary, @r###"
outA:Resolved(String)
outB:Resolved(Integer)
first:Resolved(String)
"###);
}
