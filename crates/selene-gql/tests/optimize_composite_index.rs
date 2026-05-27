//! BRIEF-29 composite-index optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    BinaryOp, EmptyProcedureRegistry, FilterPredicate, FilterPredicateKind, IndexKey, JoinTree,
    Literal, NodeOrEdgeScan, ScanAccess, ValueExpr, analyze, optimize, parse, plan,
};
use selene_testing::MockIndexCatalog;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn integer_property_names(count: usize) -> Vec<(IStr, selene_gql::IndexKind)> {
    (0..count)
        .map(|index| (istr(&format!("p{index}")), selene_gql::IndexKind::Integer))
        .collect()
}

fn equality_predicate_source(count: usize) -> String {
    (0..count)
        .map(|index| format!("n.p{index} = {index}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn match_with_equality_count(count: usize) -> String {
    format!(
        "MATCH (n:Doc) WHERE {} RETURN n",
        equality_predicate_source(count)
    )
}

fn optimized_one(source: &str, catalog: &MockIndexCatalog) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes");
    let plan = plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans");
    let ctx = selene_gql::OptimizeContext::default().with_index_catalog(catalog);
    optimize(plan, &ctx)
}

fn first_scan(tree: &JoinTree) -> Option<&NodeOrEdgeScan> {
    match tree {
        JoinTree::Scan(scan) => Some(scan),
        JoinTree::Expand { child, .. } | JoinTree::PathSearch { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

fn integer_literal(expr: &ValueExpr) -> Option<i64> {
    match expr {
        ValueExpr::Literal(Literal::Integer(value, _)) => Some(*value),
        _ => None,
    }
}

fn property_access_key(expr: &ValueExpr) -> Option<IStr> {
    let ValueExpr::PropertyAccess { key, .. } = expr else {
        return None;
    };
    Some(*key)
}

fn expression_property_integer(predicate: &FilterPredicate, expected_key: IStr) -> Option<i64> {
    let ValueExpr::BinaryOp {
        op: BinaryOp::Eq,
        lhs,
        rhs,
        ..
    } = &predicate.expr
    else {
        return None;
    };
    if property_access_key(lhs.as_ref()) == Some(expected_key) {
        return integer_literal(rhs.as_ref());
    }
    if property_access_key(rhs.as_ref()) == Some(expected_key) {
        return integer_literal(lhs.as_ref());
    }
    None
}

fn property_integer(predicate: &FilterPredicate, expected_key: IStr) -> Option<i64> {
    match &predicate.kind {
        FilterPredicateKind::PropertyEquals { key, .. } if *key == expected_key => {
            integer_literal(&predicate.expr)
        }
        FilterPredicateKind::Expression => expression_property_integer(predicate, expected_key),
        _ => None,
    }
}

fn residual_integers(scan: &NodeOrEdgeScan, key: IStr) -> Vec<i64> {
    scan.property_predicates
        .iter()
        .filter_map(|predicate| property_integer(predicate, key))
        .collect()
}

fn composite_key_integer(keys: &[(IStr, IndexKey)], key: IStr) -> Option<i64> {
    keys.iter().find_map(|(candidate_key, index_key)| {
        let IndexKey::Literal(literal) = index_key else {
            return None;
        };
        match literal {
            Literal::Integer(value, _) if *candidate_key == key => Some(*value),
            _ => None,
        }
    })
}

#[test]
fn composite_lookup_uses_declaration_order() {
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Doc"),
        vec![
            (istr("tenant"), selene_gql::IndexKind::String),
            (istr("kind"), selene_gql::IndexKind::String),
        ],
    );
    let plan = optimized_one(
        "MATCH (n:Doc) WHERE n.kind = 'k' AND n.tenant = 't1' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::CompositeLookup {
        ref properties,
        ref keys,
        ..
    } = scan.access
    else {
        panic!("expected composite lookup, got {:?}", scan.access);
    };
    assert_eq!(properties, &vec![istr("tenant"), istr("kind")]);
    assert_eq!(keys[0].0, istr("tenant"));
    assert_eq!(keys[1].0, istr("kind"));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn composite_index_lookup_dedupes_duplicate_property_keys() {
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Doc"),
        vec![
            (istr("tenant"), selene_gql::IndexKind::Integer),
            (istr("year"), selene_gql::IndexKind::Integer),
        ],
    );
    let plan = optimized_one(
        "MATCH (n:Doc) WHERE n.tenant = 1 AND n.tenant = 1 AND n.year = 2024 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::CompositeLookup {
        ref properties,
        ref keys,
        ..
    } = scan.access
    else {
        panic!("expected composite lookup, got {:?}", scan.access);
    };
    assert_eq!(properties, &vec![istr("tenant"), istr("year")]);
    assert_eq!(composite_key_integer(keys, istr("tenant")), Some(1));
    assert_eq!(composite_key_integer(keys, istr("year")), Some(2024));

    let tenant_residuals = residual_integers(scan, istr("tenant"));
    assert!(
        tenant_residuals.is_empty() || tenant_residuals == vec![1],
        "exact duplicate residual should be absent or a redundant tenant=1 predicate, got {tenant_residuals:?}"
    );
}

#[test]
fn composite_index_lookup_rewrites_scan_under_path_search_selector() {
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Doc"),
        vec![
            (istr("tenant"), selene_gql::IndexKind::String),
            (istr("kind"), selene_gql::IndexKind::String),
        ],
    );
    let plan = optimized_one(
        "MATCH ANY (n:Doc {tenant: 't1', kind: 'k'}) RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::CompositeLookup { .. }));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn composite_index_lookup_keeps_conflicting_duplicates_in_residual() {
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Doc"),
        vec![
            (istr("tenant"), selene_gql::IndexKind::Integer),
            (istr("year"), selene_gql::IndexKind::Integer),
        ],
    );
    let plan = optimized_one(
        "MATCH (n:Doc) WHERE n.tenant = 1 AND n.tenant = 2 AND n.year = 2024 RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    let ScanAccess::CompositeLookup {
        ref properties,
        ref keys,
        ..
    } = scan.access
    else {
        panic!("expected composite lookup, got {:?}", scan.access);
    };
    assert_eq!(properties, &vec![istr("tenant"), istr("year")]);
    assert_eq!(composite_key_integer(keys, istr("tenant")), Some(1));
    assert_eq!(composite_key_integer(keys, istr("year")), Some(2024));
    assert_eq!(residual_integers(scan, istr("tenant")), vec![2]);
}

#[test]
fn composite_index_lookup_does_not_panic_on_oversized_candidates() {
    let catalog = MockIndexCatalog::new();
    let source = match_with_equality_count(64);

    let result = std::panic::catch_unwind(|| optimized_one(&source, &catalog));

    assert!(result.is_ok());
    let plan = result.expect("optimizer should not panic");
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    assert!(matches!(scan.access, ScanAccess::Linear));
}

#[test]
fn composite_index_lookup_rewrites_at_cap_boundary() {
    let catalog =
        MockIndexCatalog::new().with_node_composite_index(istr("Doc"), integer_property_names(16));
    let plan = optimized_one(&match_with_equality_count(16), &catalog);
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::CompositeLookup { .. }));
    assert!(scan.property_predicates.is_empty());
}

#[test]
fn composite_index_lookup_bails_above_cap() {
    let catalog =
        MockIndexCatalog::new().with_node_composite_index(istr("Doc"), integer_property_names(17));
    let plan = optimized_one(&match_with_equality_count(17), &catalog);
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();

    assert!(matches!(scan.access, ScanAccess::Linear));
    assert_eq!(scan.property_predicates.len(), 17);
}

#[test]
fn sentinel_composite_index_snapshot() {
    let catalog = MockIndexCatalog::new().with_node_composite_index(
        istr("Doc"),
        vec![
            (istr("tenant"), selene_gql::IndexKind::String),
            (istr("kind"), selene_gql::IndexKind::String),
        ],
    );
    let plan = optimized_one(
        "MATCH (n:Doc) WHERE n.kind = 'k' AND n.tenant = 't1' RETURN n",
        &catalog,
    );
    let scan = first_scan(&plan.pattern_plan.as_ref().unwrap().join_tree).unwrap();
    let summary = match scan.access {
        ScanAccess::CompositeLookup {
            ref properties,
            ref keys,
            ..
        } => format!(
            "access=composite\nproperties={}\nkeys={}\nresidual_filters={}",
            properties
                .iter()
                .map(|property| property.as_str())
                .collect::<Vec<_>>()
                .join(","),
            keys.len(),
            scan.property_predicates.len()
        ),
        ref other => format!("unexpected={other:?}"),
    };

    insta::assert_snapshot!(summary, @r###"
access=composite
properties=tenant,kind
keys=2
residual_filters=0
"###);
}
