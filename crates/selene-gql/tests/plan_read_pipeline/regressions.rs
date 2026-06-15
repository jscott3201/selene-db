use super::*;

#[test]
fn scalar_functions_are_not_classified_as_aggregates_in_group_by() {
    // Why: prior `aggregate_name` returned Some for any single-segment
    // function call, so scalar functions like `char_length` were lifted into
    // `GroupBy.aggregates` even though they are pure scalars.
    let plan = plan_one(
        "MATCH (n) RETURN char_length(n.name) AS l, count(*) AS c GROUP BY char_length(n.name)",
    );
    let mut aggregate_names: Vec<String> = Vec::new();
    for op in &plan.pipeline {
        if let PipelineOp::GroupBy { aggregates, .. } = op {
            for aggregate in aggregates {
                aggregate_names.push(aggregate.function.as_str().to_string());
            }
        }
    }
    assert!(
        aggregate_names.iter().any(|name| name == "count"),
        "count must remain an aggregate: {aggregate_names:?}"
    );
    assert!(
        !aggregate_names.iter().any(|name| name == "char_length"),
        "char_length must not be classified as an aggregate: {aggregate_names:?}"
    );
}

#[test]
fn let_uses_dedicated_pipeline_op_so_prior_bindings_remain_visible() {
    // Why: prior code emitted `Project([x])` for `LET x = n`, dropping `n` from
    // the binding table even though analyzer semantics keep prior bindings in
    // scope. The dedicated `Let` op extends the table in place.
    let plan = plan_one("MATCH (n) LET x = n RETURN n, x");
    assert!(
        plan.pipeline
            .iter()
            .any(|op| matches!(op, PipelineOp::Let(_))),
        "LET must lower to PipelineOp::Let; got {:?}",
        variant_names(&plan)
    );
    let names: Vec<_> = plan
        .output_schema
        .columns
        .iter()
        .filter_map(|column| column.name.as_ref().map(|name| name.as_str().to_string()))
        .collect();
    assert!(
        names.iter().any(|name| name == "n"),
        "n must remain visible after LET: {names:?}"
    );
    assert!(
        names.iter().any(|name| name == "x"),
        "x must be visible after LET: {names:?}"
    );
}

#[test]
fn aggregate_only_query_emits_groupby_with_empty_keys() {
    // Why: `RETURN count(*)` without `GROUP BY` must collapse to one row.
    // Prior code only emitted `GroupBy` when `clause.group_by.is_some()`, so
    // aggregate-only queries silently degraded into per-row projection.
    let plan = plan_one("MATCH (n) RETURN count(*) AS c");
    let groupings: Vec<_> = plan
        .pipeline
        .iter()
        .filter_map(|op| {
            if let PipelineOp::GroupBy { keys, aggregates } = op {
                Some((keys.len(), aggregates.len()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        groupings,
        vec![(0, 1)],
        "expected one GroupBy with empty keys + 1 aggregate; got pipeline {:?}",
        variant_names(&plan)
    );
}

#[test]
fn explicit_empty_grouping_set_emits_groupby_with_empty_keys() {
    let plan = plan_one("FOR x IN [1, 2] RETURN count(*) AS c GROUP BY ()");
    let groupings: Vec<_> = plan
        .pipeline
        .iter()
        .filter_map(|op| {
            if let PipelineOp::GroupBy { keys, aggregates } = op {
                Some((keys.len(), aggregates.len()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        groupings,
        vec![(0, 1)],
        "expected explicit empty grouping set to lower to one GroupBy with empty keys; got pipeline {:?}",
        variant_names(&plan)
    );
}

#[test]
fn leading_optional_match_lowers_to_unit_outer() {
    let plan = plan_one("OPTIONAL MATCH (a) RETURN a");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Outer { left, key, .. } = &pattern.join_tree else {
        panic!("expected leading optional outer join");
    };
    assert!(matches!(left.as_ref(), JoinTree::Unit));
    assert!(key.is_empty());
}

#[test]
fn expanded_right_node_label_is_preserved_on_edge_match() {
    // Why: `(a)-[:K]->(b:Person)` previously dropped the `:Person` constraint
    // because `collect_node_predicates` only carried properties + inline
    // WHERE. Right-node label and property predicates now ride the EdgeMatch.
    let plan = plan_one("MATCH (a)-[:K]->(b:Person) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Expand { edge, .. } = &pattern.join_tree else {
        panic!("expected expand, got {:?}", pattern.join_tree);
    };
    assert!(
        matches!(
            &edge.right_label_predicate,
            Some(LabelExpr::Single(label)) if label.as_str() == "Person"
        ),
        "right_label_predicate missing :Person, got {:?}",
        edge.right_label_predicate
    );
}

#[test]
fn anonymous_right_node_property_predicates_attach_to_edge_match() {
    // Why: predicates on anonymous targets had nowhere to go — emitting them
    // as `PropertyEquals { binding: None, .. }` in the flat filter list lost
    // the expansion context. They now ride the EdgeMatch so the executor
    // applies them at the right scan position.
    let plan = plan_one("MATCH (a)-[:K]->({age: 18}) RETURN a");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Expand { edge, .. } = &pattern.join_tree else {
        panic!("expected expand, got {:?}", pattern.join_tree);
    };
    assert_eq!(edge.right_binding, None);
    assert_eq!(edge.right_property_predicates.len(), 1);
    assert!(
        matches!(
            &edge.right_property_predicates[0].kind,
            FilterPredicateKind::PropertyEquals { binding: None, key } if key.as_str() == "age"
        ),
        "expected anonymous-binding age predicate, got {:?}",
        edge.right_property_predicates[0].kind
    );
}

#[test]
fn sentinel_plan_shape_snapshot() {
    let plan = plan_one("MATCH (n:Person) WHERE n.age > 30 RETURN n.name AS name LIMIT 10");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let scan = match &pattern.join_tree {
        JoinTree::Scan(scan) => scan,
        other => panic!("expected scan, got {other:?}"),
    };
    let summary = format!(
        "scan_binding={}\nscan_label={}\npattern_filters={}\npipeline={:?}\ncolumns={}",
        scan.binding.is_some(),
        matches!(scan.label_predicate, Some(LabelExpr::Single(_))),
        pattern.filters.len(),
        variant_names(&plan),
        plan.output_schema.columns.len()
    );
    insta::assert_snapshot!(summary, @r###"
scan_binding=true
scan_label=true
pattern_filters=1
pipeline=["Project", "Limit"]
columns=1
"###);
}
