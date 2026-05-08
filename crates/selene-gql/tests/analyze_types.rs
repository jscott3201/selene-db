//! Analyzer type-inference tests.

use selene_core::intern;
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, ConditionClause,
    GqlStatus, GqlType, IsCheckKind, Literal, PipelineStatement, QueryPipeline, ReturnClause,
    ReturnItem, SourceSpan, Statement, TypeMismatchContext, ValueExpr, analyze, parse,
};

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement)
}

fn projection_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    let item = query
        .statements
        .iter()
        .filter_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(&clause.items),
            PipelineStatement::With(clause) => Some(&clause.items),
            _ => None,
        })
        .flatten()
        .find(|item| {
            item.alias.is_some_and(|alias| alias.as_str() == name)
                || matches!(&item.expr, ValueExpr::Variable { name: value, .. } if value.as_str() == name)
        })
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
}

#[test]
fn integer_arithmetic_promotes_to_integer() {
    let analyzed = analyze_one("RETURN 1 + 2 AS sum").unwrap();
    assert_eq!(
        projection_type(&analyzed, "sum"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn float_plus_integer_promotes_to_float64() {
    let analyzed = analyze_one("RETURN 1 + 2.0 AS sum").unwrap();
    assert_eq!(
        projection_type(&analyzed, "sum"),
        AnalyzedType::Resolved(GqlType::Float64)
    );
}

#[test]
fn pattern_node_is_node_ref() {
    let analyzed = analyze_one("MATCH (n) RETURN n").unwrap();
    assert_eq!(
        projection_type(&analyzed, "n"),
        AnalyzedType::Resolved(GqlType::NodeRef)
    );
}

#[test]
fn static_case_branches_unify_to_string() {
    let analyzed =
        analyze_one("RETURN CASE WHEN true THEN 'adult' ELSE 'minor' END AS label").unwrap();
    assert_eq!(
        projection_type(&analyzed, "label"),
        AnalyzedType::Resolved(GqlType::String)
    );
}

#[test]
fn dynamic_case_branch_defers_result_type() {
    let analyzed =
        analyze_one("MATCH (n) RETURN CASE WHEN n.age > 18 THEN n.name ELSE 'minor' END AS label")
            .unwrap();
    assert_eq!(projection_type(&analyzed, "label"), AnalyzedType::Dynamic);
}

#[test]
fn parameter_stays_dynamic() {
    let analyzed = analyze_one("RETURN $name AS who").unwrap();
    assert_eq!(projection_type(&analyzed, "who"), AnalyzedType::Dynamic);
}

#[test]
fn function_call_stays_dynamic_until_brief_23() {
    let analyzed = analyze_one("RETURN size([1,2,3]) AS n").unwrap();
    assert_eq!(projection_type(&analyzed, "n"), AnalyzedType::Dynamic);
}

#[test]
fn count_subquery_is_integer() {
    let analyzed = analyze_one("MATCH (n) RETURN COUNT { MATCH (n)-[:K]->(m) } AS c").unwrap();
    assert_eq!(
        projection_type(&analyzed, "c"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn unwind_list_aliases_to_element_type() {
    let analyzed = analyze_one("UNWIND [1, 2, 3] AS x RETURN x").unwrap();
    assert_eq!(
        projection_type(&analyzed, "x"),
        AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn record_literal_stays_dynamic() {
    let analyzed = analyze_one("RETURN {score: 1} AS r").unwrap();
    assert_eq!(projection_type(&analyzed, "r"), AnalyzedType::Dynamic);
}

#[test]
fn expr_type_table_is_deterministic_for_same_source() {
    let left = analyze_one("RETURN 1 + 2 AS sum").unwrap();
    let right = analyze_one("RETURN 1 + 2 AS sum").unwrap();
    let left_types = left
        .expr_types
        .iter()
        .map(|(_, ty)| ty.clone())
        .collect::<Vec<_>>();
    let right_types = right
        .expr_types
        .iter()
        .map(|(_, ty)| ty.clone())
        .collect::<Vec<_>>();

    assert_eq!(left.expr_types.len(), 3);
    assert_eq!(left_types, right_types);
}

#[test]
fn add_string_and_integer_errors() {
    let err = analyze_one("RETURN 'a' + 1 AS x").unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::BinaryArithmetic { .. },
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
}

#[test]
fn not_on_integer_errors() {
    let err = analyze_one("RETURN NOT 1 AS x").unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::UnaryNot,
            ..
        }
    ));
}

#[test]
fn compare_boolean_to_integer_errors() {
    let err = analyze_one("RETURN true < 1 AS x").unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::BinaryComparison { .. },
            ..
        }
    ));
}

#[test]
fn case_branches_with_disjoint_types_error() {
    let err = analyze_one("MATCH (n) RETURN CASE WHEN n.age > 18 THEN 1 ELSE 'minor' END AS label")
        .unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::CaseBranchUnification,
            ..
        }
    ));
}

#[test]
fn list_literal_with_disjoint_types_errors() {
    let err = analyze_one("RETURN [1, 'a'] AS xs").unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::ListLiteralUnification,
            ..
        }
    ));
}

#[test]
fn boolean_conditions_reject_static_non_boolean() {
    for (source, expected_clause) in [
        ("MATCH (n) WHERE 1 RETURN n", ConditionClause::MatchWhere),
        ("MATCH (n WHERE 1) RETURN n", ConditionClause::InlineWhere),
        ("MATCH (n) FILTER 1 RETURN n", ConditionClause::Filter),
        (
            "MATCH (n) RETURN n GROUP BY n HAVING 1",
            ConditionClause::Having,
        ),
        (
            "MATCH (n) WITH n AS kept WHERE 1 RETURN kept",
            ConditionClause::WithWhere,
        ),
        (
            "RETURN CASE WHEN 1 THEN 'yes' ELSE 'no' END AS answer",
            ConditionClause::CaseWhen,
        ),
    ] {
        let err = analyze_one(source).unwrap_err();
        assert!(
            matches!(
                err,
                AnalysisError::TypeMismatch {
                    context: TypeMismatchContext::Condition { clause },
                    ..
                } if clause == expected_clause
            ),
            "{source} produced {err:?}"
        );
    }
}

#[test]
fn is_typed_unsupported_variant_errors_for_hand_built_ast() {
    let span = SourceSpan::new(0, 1);
    let expr = ValueExpr::IsCheck {
        operand: Box::new(ValueExpr::Literal(Literal::Null(span))),
        kind: IsCheckKind::Typed(GqlType::GraphRef),
        negated: false,
        span,
    };
    let statement = Statement::Query(QueryPipeline {
        statements: vec![PipelineStatement::Return(ReturnClause {
            distinct: false,
            star: false,
            items: vec![ReturnItem {
                expr,
                alias: Some(intern("ok").unwrap()),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    });

    let err = analyze(statement).unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::IsTypedTarget,
            ..
        }
    ));
}
