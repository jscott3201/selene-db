//! BRIEF-28 constant-folding optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    AnalyzedStatement, BinaryOp, CatalogOp, EmptyProcedureRegistry, Literal, MutationOp,
    PipelineOp, PlannedTypePropertyConstraint, ProcedureOutputColumn, SourceSpan, ValueExpr,
    analyze, optimize, parse, plan,
};
use selene_testing::MockProcedureRegistry;

fn analyzed(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes")
}

fn optimized_one(source: &str) -> selene_gql::ExecutionPlan {
    let analyzed = analyzed(source);
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    optimize(plan, &selene_gql::OptimizeContext::default())
}

fn optimized_with_registry(
    source: &str,
    registry: &MockProcedureRegistry,
) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    let plan = plan(&analyzed, registry).expect("test input plans");
    optimize(plan, &selene_gql::OptimizeContext::default())
}

fn project_expr(plan: &selene_gql::ExecutionPlan) -> &ValueExpr {
    plan.pipeline
        .iter()
        .find_map(|op| match op {
            PipelineOp::Project(items) => Some(&items[0].expr),
            _ => None,
        })
        .expect("project expression")
}

fn istr(value: &str) -> IStr {
    intern(value).expect("test interner")
}

#[test]
fn folds_project_arithmetic_bottom_up() {
    let plan = optimized_one("RETURN 1 + 2 * 3 AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::Integer(7, _))
    ));
}

#[test]
fn folds_boolean_unary_and_string_concat() {
    let plan = optimized_one("RETURN NOT TRUE AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::Bool(false, _))
    ));

    let plan = optimized_one("RETURN 'foo' || 'bar' AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::String(value, _)) if value.as_str() == "foobar"
    ));
}

#[test]
fn folds_temporal_literal_comparisons() {
    let plan = optimized_one("RETURN DATE '2026-05-07' < DATE '2026-05-08' AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::Bool(true, _))
    ));

    let plan = optimized_one(
        "RETURN LOCAL DATETIME '2026-05-07T12:34:56' = LOCAL DATETIME '2026-05-07T12:34:56' AS x",
    );
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::Bool(true, _))
    ));

    let plan = optimized_one("RETURN LOCAL TIME '12:34:56' >= LOCAL TIME '12:34:55' AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::Literal(Literal::Bool(true, _))
    ));
}

#[test]
fn preserves_runtime_error_and_overflow_expressions() {
    let plan = optimized_one("RETURN 1 / 0 AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::BinaryOp {
            op: selene_gql::BinaryOp::Div,
            ..
        }
    ));

    let plan = optimized_one("RETURN 9223372036854775807 + 1 AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::BinaryOp {
            op: selene_gql::BinaryOp::Add,
            ..
        }
    ));
}

#[test]
fn literal_safe_short_circuit_keeps_non_literal_operand() {
    let plan = optimized_one("MATCH (n) RETURN FALSE AND n.flag AS x");
    assert!(matches!(
        project_expr(&plan),
        ValueExpr::BinaryOp {
            op: selene_gql::BinaryOp::And,
            ..
        }
    ));
}

#[test]
fn folds_pattern_property_and_where_expressions() {
    let plan = optimized_one("MATCH (n {age: 1 + 2}) WHERE 3 = 1 + 2 RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let selene_gql::JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert!(matches!(
        scan.property_predicates[0].expr,
        ValueExpr::Literal(Literal::Integer(3, _))
    ));
    assert!(matches!(
        pattern.filters[0].expr,
        ValueExpr::Literal(Literal::Bool(true, _))
    ));
}

#[test]
fn folds_call_mutation_and_catalog_expression_payloads() {
    let registry = MockProcedureRegistry::new().with_procedure(
        vec![istr("pkg"), istr("args")],
        vec![selene_gql::ProcedureParameter::new(
            istr("a"),
            selene_gql::GqlType::Integer,
            false,
        )],
        vec![ProcedureOutputColumn::new(
            istr("out"),
            selene_gql::GqlType::Integer,
        )],
    );
    let plan = optimized_with_registry("CALL pkg.args(1 + 2)", &registry);
    let [PipelineOp::Call(call)] = plan.pipeline.as_slice() else {
        panic!("expected call");
    };
    assert!(matches!(
        call.args[0].expr,
        ValueExpr::Literal(Literal::Integer(3, _))
    ));

    let plan = optimized_one("INSERT (:Person {age: 1 + 2})");
    assert!(plan.pipeline.iter().any(|op| matches!(
        op,
        PipelineOp::Mutation(MutationOp::InsertNode { property_inits, .. })
            if matches!(property_inits[0].value.expr, ValueExpr::Literal(Literal::Integer(3, _)))
    )));

    let mut plan = {
        let analyzed = analyzed("CREATE NODE TYPE :Person (age :: INT DEFAULT 0)");
        selene_gql::plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
    };
    let [PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. })] =
        plan.pipeline.as_mut_slice()
    else {
        panic!("expected catalog create node type");
    };
    let [PlannedTypePropertyConstraint::Default(project, _)] =
        properties[0].constraints.as_mut_slice()
    else {
        panic!("expected default constraint");
    };
    project.expr = ValueExpr::BinaryOp {
        op: BinaryOp::Add,
        lhs: Box::new(ValueExpr::Literal(Literal::Integer(
            1,
            SourceSpan::new(0, 1),
        ))),
        rhs: Box::new(ValueExpr::Literal(Literal::Integer(
            2,
            SourceSpan::new(4, 1),
        ))),
        span: SourceSpan::new(0, 5),
    };
    let plan = optimize(plan, &selene_gql::OptimizeContext::default());
    let [PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. })] =
        plan.pipeline.as_slice()
    else {
        panic!("expected catalog create node type");
    };
    assert!(matches!(
        properties[0].constraints.as_slice(),
        [PlannedTypePropertyConstraint::Default(project, _)]
            if matches!(project.expr, ValueExpr::Literal(Literal::Integer(3, _)))
    ));
}

#[test]
fn sentinel_constant_folding_snapshot() {
    let plan = optimized_one("MATCH (n {age: 1 + 2}) RETURN 1 + 2 * 3 AS x");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let selene_gql::JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    let summary = format!(
        "project={}\nproperty={}",
        literal_summary(project_expr(&plan)),
        literal_summary(&scan.property_predicates[0].expr),
    );
    insta::assert_snapshot!(summary, @r###"
project=Integer(7)
property=Integer(3)
"###);
}

fn literal_summary(expr: &ValueExpr) -> String {
    match expr {
        ValueExpr::Literal(Literal::Bool(value, _)) => format!("Bool({value})"),
        ValueExpr::Literal(Literal::Integer(value, _)) => format!("Integer({value})"),
        ValueExpr::Literal(Literal::Float(value, _)) => format!("Float({value})"),
        ValueExpr::Literal(Literal::String(value, _)) => format!("String({})", value.as_str()),
        ValueExpr::Literal(Literal::Null(_)) => "Null".to_string(),
        other => format!("{other:?}"),
    }
}
