//! BRIEF-29 composite-index optimizer tests.

use selene_core::{IStr, intern};
use selene_gql::{
    EmptyProcedureRegistry, JoinTree, NodeOrEdgeScan, ScanAccess, analyze, optimize, parse, plan,
};
use selene_testing::MockIndexCatalog;

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
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
        JoinTree::Expand { child, .. } => first_scan(child),
        JoinTree::HashJoin { left, right, .. } | JoinTree::Outer { left, right, .. } => {
            first_scan(left).or_else(|| first_scan(right))
        }
        JoinTree::WorstCaseOptimal { .. } | JoinTree::Subplan(_) => None,
        _ => None,
    }
}

#[test]
fn composite_lookup_uses_declaration_order() {
    let catalog = MockIndexCatalog::new()
        .with_node_composite_index(istr("Doc"), vec![istr("tenant"), istr("kind")]);
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
fn sentinel_composite_index_snapshot() {
    let catalog = MockIndexCatalog::new()
        .with_node_composite_index(istr("Doc"), vec![istr("tenant"), istr("kind")]);
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
