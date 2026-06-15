use super::*;

#[test]
fn expr_type_table_is_deterministic_for_same_source() {
    let left = analyze_one("RETURN 1 + 2 AS \"sum\"").unwrap();
    let right = analyze_one("RETURN 1 + 2 AS \"sum\"").unwrap();
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
    let analyzed = analyze_one("RETURN 1 + 2 AS \"sum\"").unwrap();
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
