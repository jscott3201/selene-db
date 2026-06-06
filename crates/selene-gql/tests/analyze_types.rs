//! Analyzer type-inference tests.

use selene_core::db_string;
use selene_gql::{
    AnalysisError, AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, ConditionClause,
    EmptyProcedureRegistry, GqlStatus, GqlType, IsCheckKind, Literal, PipelineStatement,
    QueryPipeline, ReturnClause, ReturnItem, Side, SourceSpan, Statement, TypeMismatchContext,
    ValueExpr, analyze, parse,
};

fn analyze_one(source: &str) -> Result<AnalyzedStatement, AnalysisError> {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None)
}

fn type_mismatch(source: &str) -> (TypeMismatchContext, SourceSpan) {
    let err = analyze_one(source).unwrap_err();
    match err {
        AnalysisError::TypeMismatch { context, span, .. } => (context, span),
        other => panic!("expected TypeMismatch for {source}, got {other:?}"),
    }
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
            item.alias.clone().is_some_and(|alias| alias.as_str() == name)
                || matches!(&item.expr, ValueExpr::Variable { name: value, .. } if value.as_str() == name)
        })
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
}

fn return_items(analyzed: &AnalyzedStatement) -> &[ReturnItem] {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    query
        .statements
        .iter()
        .find_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(clause.items.as_slice()),
            _ => None,
        })
        .expect("RETURN clause exists")
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
fn quantified_edge_binding_is_edge_ref_list() {
    let analyzed = analyze_one("MATCH (a)-[r:K*1..2]->(b) RETURN r").unwrap();
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::List(Box::new(GqlType::EdgeRef)))
    );
}

#[test]
fn group_variable_property_access_is_dynamic() {
    let analyzed = analyze_one("MATCH (a)-[r:K*1..2]->(b) RETURN r.weight AS weights").unwrap();
    assert_eq!(projection_type(&analyzed, "weights"), AnalyzedType::Dynamic);
}

#[test]
fn group_variable_property_exists_is_rejected() {
    let err = analyze_one("MATCH (a)-[r:K*1..2]->(b) RETURN PROPERTY_EXISTS(r, 'weight')")
        .expect_err("PROPERTY_EXISTS over graph-element lists remains unsupported");
    assert!(matches!(err, AnalysisError::NotImplemented { .. }));
    assert_eq!(err.gqlstatus().as_str(), "42N01");
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
fn function_call_expression_stays_dynamic_until_scalar_dispatch() {
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
fn record_literal_resolves_to_open_record() {
    // An open `RECORD{...}` value literal resolves to the open record type
    // (ISO feature GV45, `<record constructor>` clause 20.18). `RecordType::Open`
    // is a pure tag with no per-field inference; the executor builds the open
    // record at runtime.
    let analyzed = analyze_one("RETURN {score: 1} AS r").unwrap();
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Record(selene_gql::RecordType::Open))
    );
}

#[test]
fn list_concat_returns_list_type() {
    let analyzed = analyze_one("RETURN [1] || [2] AS xs").unwrap();
    assert_eq!(
        projection_type(&analyzed, "xs"),
        AnalyzedType::Resolved(GqlType::List(Box::new(GqlType::Integer)))
    );
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
fn expr_id_lookup_distinguishes_repeated_structural_occurrences() {
    let analyzed = analyze_one("RETURN 1 + 1 AS a, 1 + 1 AS b").unwrap();
    let items = return_items(&analyzed);
    let first = analyzed
        .expr_ids
        .get(&items[0].expr)
        .expect("first expression has ExprId");
    let second = analyzed
        .expr_ids
        .get(&items[1].expr)
        .expect("second expression has ExprId");

    assert_ne!(first, second);
    assert_eq!(
        analyzed.expr_types.get(first),
        &AnalyzedType::Resolved(GqlType::Integer)
    );
    assert_eq!(
        analyzed.expr_types.get(second),
        &AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn analyzed_statement_clone_preserves_expr_id_lookup() {
    let analyzed = analyze_one("RETURN 1 + 2 AS sum").unwrap();
    let cloned = analyzed.clone();
    let item = &return_items(&cloned)[0];
    let id = cloned
        .expr_ids
        .get(&item.expr)
        .expect("cloned expression lookup preserves ExprId");

    assert_eq!(
        cloned.expr_types.get(id),
        &AnalyzedType::Resolved(GqlType::Integer)
    );
}

#[test]
fn expr_id_lookup_survives_simple_case_base_clone() {
    let analyzed = analyze_one(
        "MATCH (n) RETURN CASE n.age WHEN 1 THEN 'a' WHEN 2 THEN 'b' ELSE 'c' END AS label",
    )
    .expect("analyzes");
    let items = return_items(&analyzed);
    let ValueExpr::Case { branches, .. } = &items[0].expr else {
        panic!("expected CASE expression, got {:?}", items[0].expr);
    };

    let mut ids = Vec::new();
    for (condition, _) in branches {
        let ValueExpr::BinaryOp { lhs, .. } = condition else {
            panic!("expected BinaryOp condition, got {condition:?}");
        };
        let id = analyzed
            .expr_ids
            .get(lhs)
            .expect("cloned CASE base resolves to an ExprId");
        ids.push(id);
    }

    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
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
fn compare_boolean_literals_analyzes_as_boolean() {
    let analyzed = analyze_one("RETURN false < true AS x").expect("BOOLEAN comparison analyzes");
    assert_eq!(
        projection_type(&analyzed, "x"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn compare_uuid_literals_analyzes_as_boolean() {
    let analyzed = analyze_one(
        "RETURN UUID '00000000-0000-0000-0000-000000000001' \
         < UUID '00000000-0000-0000-0000-000000000002' AS x",
    )
    .expect("UUID comparison analyzes");
    assert_eq!(
        projection_type(&analyzed, "x"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}

#[test]
fn compare_graph_references_analyzes_by_static_base_type() {
    let node = analyze_one("MATCH (a), (b) RETURN a < b AS x").expect("NODE comparison analyzes");
    assert_eq!(
        projection_type(&node, "x"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );

    let edge = analyze_one("MATCH ()-[a]->(), ()-[b]->() RETURN a < b AS x")
        .expect("EDGE comparison analyzes");
    assert_eq!(
        projection_type(&edge, "x"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );

    let err = analyze_one("MATCH (a)-[b]->() RETURN a < b AS x").unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::BinaryComparison { .. },
            ..
        }
    ));
}

#[test]
fn boolean_operator_rejects_static_non_boolean_operand() {
    let (context, _) = type_mismatch("RETURN true AND 1 AS x");
    assert!(matches!(
        context,
        TypeMismatchContext::BinaryBoolean {
            op: _,
            side: Side::Rhs
        }
    ));
}

#[test]
fn concat_rejects_static_non_list_or_string_operand() {
    let (context, _) = type_mismatch("RETURN 'a' || 1 AS x");
    assert!(matches!(
        context,
        TypeMismatchContext::BinaryConcat { side: Side::Rhs }
    ));
}

#[test]
fn string_predicate_rejects_static_non_string_operand() {
    let (context, _) = type_mismatch("RETURN 1 CONTAINS 'a' AS x");
    assert!(matches!(
        context,
        TypeMismatchContext::BinaryStringPredicate {
            op: _,
            side: Side::Lhs
        }
    ));
}

#[test]
fn is_normalized_uses_dedicated_mismatch_context() {
    let (context, _) = type_mismatch("RETURN 1 IS NORMALIZED AS x");
    assert!(matches!(context, TypeMismatchContext::IsNormalized));
}

#[test]
fn unary_negate_rejects_static_non_numeric_operand() {
    let (context, _) = type_mismatch("RETURN -true AS x");
    assert!(matches!(context, TypeMismatchContext::UnaryNegate));
}

#[test]
fn in_list_updates_unified_type_across_resolved_items() {
    let (context, _) = type_mismatch("RETURN 'a' IN [NULL, 'b', 1] AS x");
    assert!(matches!(context, TypeMismatchContext::InListUnification));
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
fn case_dynamic_branch_still_validates_resolved_branches() {
    let (context, span) = type_mismatch(
        "MATCH (n) RETURN CASE WHEN true THEN n.age WHEN false THEN 'a' ELSE 1 END AS label",
    );
    assert!(matches!(
        context,
        TypeMismatchContext::CaseBranchUnification
    ));
    assert!(span.byte_len > 0);
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
fn list_dynamic_item_still_validates_resolved_items() {
    let (context, span) = type_mismatch("MATCH (n) RETURN [n.age, 'a', 1] AS xs");
    assert!(matches!(
        context,
        TypeMismatchContext::ListLiteralUnification
    ));
    assert!(span.byte_len > 0);
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
                alias: Some(db_string("ok").unwrap()),
                span,
            }],
            group_by: None,
            having: None,
            span,
        })],
        span,
    });

    let err = analyze(statement, &EmptyProcedureRegistry, None).unwrap_err();
    assert!(matches!(
        err,
        AnalysisError::TypeMismatch {
            context: TypeMismatchContext::IsTypedTarget,
            ..
        }
    ));
}

// ANALYZE-01: the analyzer must NOT statically reject a literal `NULL` operand
// in comparison / arithmetic / boolean operators. Per ISO/IEC 39075:2024 §12.x
// three-valued logic, those operators yield NULL (UNKNOWN) when an operand is
// NULL; the runtime already does so. A static type rejection there is a
// conformance bug (a query the engine can evaluate fails analysis).

#[test]
fn analyze_accepts_null_comparison_operand() {
    let analyzed = analyze_one("RETURN NULL < 5 AS r").expect("NULL < 5 analyzes");
    // Comparison with a known operand still resolves to Boolean (the NULL truth
    // value is a Boolean-domain result, surfaced at runtime as NULL).
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    // Symmetric: NULL on the right of a literal.
    analyze_one("RETURN 5 < NULL AS r").expect("5 < NULL analyzes");
    // NULL on both sides.
    analyze_one("RETURN NULL < NULL AS r").expect("NULL < NULL analyzes");
}

#[test]
fn analyze_accepts_null_arithmetic_operand() {
    // One concrete operand makes the static result Dynamic (the NULL operand has
    // no numeric kind to promote against); the point is that it does NOT error.
    let analyzed = analyze_one("RETURN NULL + 1 AS r").expect("NULL + 1 analyzes");
    assert_eq!(projection_type(&analyzed, "r"), AnalyzedType::Dynamic);
    analyze_one("RETURN 1 + NULL AS r").expect("1 + NULL analyzes");
    analyze_one("RETURN NULL * 2 AS r").expect("NULL * 2 analyzes");
}

#[test]
fn analyze_accepts_unary_negate_over_null() {
    let analyzed = analyze_one("RETURN -NULL AS r").expect("-NULL analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Null)
    );
}

#[test]
fn analyze_accepts_null_boolean_operand() {
    let analyzed = analyze_one("RETURN NULL AND true AS r").expect("NULL AND true analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
    analyze_one("RETURN true OR NULL AS r").expect("true OR NULL analyzes");
    analyze_one("RETURN NULL XOR NULL AS r").expect("NULL XOR NULL analyzes");
}

#[test]
fn analyze_accepts_unary_not_over_null() {
    let analyzed = analyze_one("RETURN NOT NULL AS r").expect("NOT NULL analyzes");
    assert_eq!(
        projection_type(&analyzed, "r"),
        AnalyzedType::Resolved(GqlType::Boolean)
    );
}
