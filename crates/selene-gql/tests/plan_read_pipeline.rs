//! BRIEF-26 read-side planner lowering tests.

use selene_gql::{
    AnalyzedStatement, BindingElement, EdgeDirection, EmptyProcedureRegistry, FilterPredicateKind,
    JoinTree, LabelExpr, LimitAmount, PipelineOp, PlannerError, ScanKind, SetOp, analyze, parse,
    plan,
};

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test input parses");
    analyze(statement, &EmptyProcedureRegistry, None).expect("test input analyzes")
}

fn plan_one(source: &str) -> selene_gql::ExecutionPlan {
    let analyzed = analyze_one(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect("test input plans")
}

fn plan_err(source: &str) -> PlannerError {
    let analyzed = analyze_one(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect_err("test input should fail planning")
}

fn variant_names(plan: &selene_gql::ExecutionPlan) -> Vec<&'static str> {
    plan.pipeline
        .iter()
        .map(|op| match op {
            PipelineOp::Filter(_) => "Filter",
            PipelineOp::Project(_) => "Project",
            PipelineOp::Let(_) => "Let",
            PipelineOp::Unwind { .. } => "Unwind",
            PipelineOp::OrderBy(_) => "OrderBy",
            PipelineOp::Limit { .. } => "Limit",
            PipelineOp::TopK { .. } => "TopK",
            PipelineOp::GroupBy { .. } => "GroupBy",
            PipelineOp::Distinct => "Distinct",
            PipelineOp::Union { .. } => "Union",
            PipelineOp::Chain(_) => "Chain",
            PipelineOp::Call(_) => "Call",
            PipelineOp::Mutation(_) => "Mutation",
            PipelineOp::Catalog(_) => "Catalog",
            PipelineOp::Tx(_) => "Tx",
            _ => "Unknown",
        })
        .collect()
}

fn expand(plan: &selene_gql::ExecutionPlan) -> (&selene_gql::EdgeMatch, EdgeDirection) {
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    match &pattern.join_tree {
        JoinTree::Expand {
            edge, direction, ..
        } => (edge, *direction),
        other => panic!("expected expand, got {other:?}"),
    }
}

#[test]
fn lowers_match_return_scan() {
    let plan = plan_one("MATCH (n) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.bindings.len(), 1);
    assert_eq!(pattern.bindings[0].name.as_str(), "n");
    assert_eq!(pattern.bindings[0].element, BindingElement::Node);
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.kind, ScanKind::Node);
    assert!(scan.binding.is_some());
    assert_eq!(variant_names(&plan), ["Project"]);
    assert_eq!(plan.output_schema.columns[0].name.unwrap().as_str(), "n");
}

#[test]
fn anonymous_node_scan_has_no_binding() {
    let plan = plan_one("MATCH (:Person) RETURN 1");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.binding, None);
    assert!(matches!(
        scan.label_predicate,
        Some(LabelExpr::Single(label)) if label.as_str() == "Person"
    ));
}

#[test]
fn anonymous_edge_endpoints_are_preserved_as_none() {
    let plan = plan_one("MATCH ()-[e]->() RETURN e");
    let (edge, direction) = expand(&plan);
    assert_eq!(direction, EdgeDirection::Right);
    assert!(edge.binding.is_some());
    assert_eq!(edge.left_binding, None);
    assert_eq!(edge.right_binding, None);
}

#[test]
fn direction_matrix_preserves_pattern_direction() {
    let right = plan_one("MATCH (a)-[:K]->(b) RETURN a, b");
    assert_eq!(expand(&right).1, EdgeDirection::Right);

    let left = plan_one("MATCH (a)<-[:K]-(b) RETURN a, b");
    assert_eq!(expand(&left).1, EdgeDirection::Left);

    let undirected = plan_one("MATCH (a)-[:K]-(b) RETURN a, b");
    assert_eq!(expand(&undirected).1, EdgeDirection::Undirected);
}

#[test]
fn optional_match_lowers_to_outer_join() {
    let plan = plan_one("MATCH (a) OPTIONAL MATCH (a)-[:K]->(b) RETURN a, b");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert!(matches!(pattern.join_tree, JoinTree::Outer { .. }));
}

#[test]
fn path_binding_creates_path_plan_placeholder() {
    let plan = plan_one("MATCH p = (a)-[:K]->(b) RETURN p");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    assert_eq!(pattern.paths.len(), 1);
}

#[test]
fn property_maps_and_inline_where_are_preserved() {
    let plan = plan_one("MATCH (n:Person {age: 42} WHERE n.active) RETURN n");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Scan(scan) = &pattern.join_tree else {
        panic!("expected scan");
    };
    assert_eq!(scan.property_predicates.len(), 1);
    assert!(matches!(
        scan.property_predicates[0].kind,
        FilterPredicateKind::PropertyEquals { key, .. } if key.as_str() == "age"
    ));
    assert_eq!(pattern.filters.len(), 1);
}

#[test]
fn preserves_set_op_in_composite_plan() {
    let plan = plan_one("RETURN 1 AS n UNION ALL RETURN 2 AS n");
    let Some(PipelineOp::Union { op, .. }) = plan.pipeline.last() else {
        panic!("expected union op");
    };
    assert_eq!(*op, SetOp::UnionAll);
}

#[test]
fn composite_subplans_keep_pattern_bindings_local() {
    let plan = plan_one("MATCH (n) RETURN n UNION MATCH (m) RETURN m");
    let first_pattern = plan.pattern_plan.as_ref().expect("first pattern");
    assert_eq!(first_pattern.bindings.len(), 1);
    assert_eq!(first_pattern.bindings[0].name.as_str(), "n");
    let Some(PipelineOp::Union { rhs, .. }) = plan.pipeline.last() else {
        panic!("expected union op");
    };
    let rhs_pattern = rhs.pattern_plan.as_ref().expect("rhs pattern");
    assert_eq!(rhs_pattern.bindings.len(), 1);
    assert_eq!(rhs_pattern.bindings[0].name.as_str(), "m");
}

#[test]
fn chained_query_uses_chain_pipeline_op() {
    let plan = plan_one("RETURN 1 AS n NEXT RETURN 2 AS n");
    assert!(matches!(plan.pipeline.last(), Some(PipelineOp::Chain(_))));
}

#[test]
fn limit_parameter_survives_to_plan() {
    let plan = plan_one("RETURN 1 AS n LIMIT $rows");
    let Some(PipelineOp::Limit { count, .. }) = plan.pipeline.last() else {
        panic!("expected limit");
    };
    assert!(matches!(count, LimitAmount::Parameter(name) if name.as_str() == "rows"));
}

#[test]
fn unaliased_derived_projection_column_has_no_name() {
    let plan = plan_one("MATCH (n) RETURN n.name");
    assert_eq!(plan.output_schema.columns.len(), 1);
    assert_eq!(plan.output_schema.columns[0].name, None);
}

#[test]
fn return_star_keeps_visible_binding_columns() {
    let plan = plan_one("MATCH (n) RETURN *");
    assert!(
        plan.output_schema
            .columns
            .iter()
            .any(|column| column.name.is_some_and(|name| name.as_str() == "n"))
    );
}

#[test]
fn order_key_carries_binding_refs() {
    let plan = plan_one("MATCH (n) RETURN n ORDER BY n.name");
    let Some(PipelineOp::OrderBy(keys)) = plan.pipeline.last() else {
        panic!("expected order-by");
    };
    assert_eq!(keys[0].binding_refs.len(), 1);
}

#[test]
fn non_leading_match_is_not_implemented() {
    let err = plan_err("MATCH (a) WITH a AS x MATCH (b) RETURN x, b");
    assert!(matches!(
        err,
        PlannerError::NotImplemented {
            feature: "non-leading MATCH (post-pipeline-boundary pattern)",
            ..
        }
    ));
}

#[test]
fn deferred_pattern_features_have_stable_tags() {
    let cases = [
        (
            "MATCH (a)-[:K*1..2]->(b) RETURN a",
            "variable-length edge patterns (quantifier)",
        ),
        (
            "MATCH ANY (n) RETURN n",
            "MATCH path selector (SHORTEST/ALL/ANY)",
        ),
        (
            "MATCH DIFFERENT EDGES (n) RETURN n",
            "MATCH mode (REPEATABLE ELEMENTS / DIFFERENT EDGES)",
        ),
        (
            "MATCH SIMPLE (n) RETURN n",
            "MATCH path mode (TRAIL/SIMPLE/ACYCLIC)",
        ),
    ];

    for (source, expected) in cases {
        let err = plan_err(source);
        assert!(
            matches!(err, PlannerError::NotImplemented { feature, .. } if feature == expected),
            "{source} should report {expected}, got {err:?}"
        );
    }
}

#[test]
fn planner_errors_emit_xx500() {
    let err = plan_err("MATCH ANY (n) RETURN n");
    assert_eq!(err.gqlstatus().as_str(), "XX500");
}

#[test]
fn return_star_after_with_uses_with_projection() {
    // Why: prior planner walked `analyzed.scopes.declarations()` for RETURN *,
    // leaking bindings discarded by a WITH boundary into the output schema.
    let plan = plan_one("MATCH (n) WITH n AS x RETURN *");
    let names: Vec<_> = plan
        .output_schema
        .columns
        .iter()
        .filter_map(|column| column.name.map(|name| name.as_str().to_string()))
        .collect();
    assert!(
        names.iter().any(|name| name == "x"),
        "x must remain visible after WITH: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name == "n"),
        "n must not leak past WITH boundary: {names:?}"
    );
}

#[test]
fn return_star_does_not_emit_empty_project() {
    // Why: RETURN * carries empty `clause.items`, so the prior unconditional
    // `Project(project_items(...))` push produced an empty projection that an
    // executor would interpret as "drop all columns".
    let plan = plan_one("MATCH (n) RETURN *");
    let empty_projects = plan
        .pipeline
        .iter()
        .filter(|op| matches!(op, PipelineOp::Project(items) if items.is_empty()))
        .count();
    assert_eq!(empty_projects, 0, "got pipeline {:?}", variant_names(&plan));
    assert!(
        plan.output_schema
            .columns
            .iter()
            .any(|column| column.name.is_some_and(|name| name.as_str() == "n")),
        "RETURN * output_schema must still expose visible bindings"
    );
}

#[test]
fn chained_plan_output_schema_uses_last_block() {
    // Why: NEXT establishes a fresh binding scope, so the outer plan's
    // output_schema must reflect the final block's projection — not the
    // first block's, which was the prior behavior.
    let plan = plan_one("RETURN 1 AS a NEXT RETURN 2 AS b");
    let names: Vec<_> = plan
        .output_schema
        .columns
        .iter()
        .filter_map(|column| column.name.map(|name| name.as_str().to_string()))
        .collect();
    assert_eq!(names, vec!["b".to_string()]);
}

#[test]
fn limit_then_offset_fuses_into_single_op() {
    // Why: parsers can emit either source order. `LIMIT n OFFSET m` previously
    // lowered to two sequential Limit ops, which means "take n then skip m" —
    // the inverse of GQL semantics.
    let plan = plan_one("RETURN 1 AS n LIMIT 10 OFFSET 5");
    let limits: Vec<_> = plan
        .pipeline
        .iter()
        .filter_map(|op| {
            if let PipelineOp::Limit { offset, count } = op {
                Some((offset.clone(), count.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(limits.len(), 1, "got pipeline {:?}", variant_names(&plan));
    assert_eq!(limits[0].0, LimitAmount::Literal(5));
    assert_eq!(limits[0].1, LimitAmount::Literal(10));
}

#[test]
fn plan_records_next_pipeline_op_id_high_water_mark() {
    let plan = plan_one("RETURN 1 AS n LIMIT 10");

    assert_eq!(plan.next_pipeline_op_id.get(), plan.pipeline.len() as u32);
}

#[test]
fn composite_plan_refreshes_next_pipeline_op_id_after_union_append() {
    let plan = plan_one("RETURN 1 AS n UNION ALL RETURN 2 AS n");

    assert_eq!(plan.next_pipeline_op_id.get(), plan.pipeline.len() as u32);
    let Some(PipelineOp::Union { rhs, .. }) = plan.pipeline.last() else {
        panic!("expected union op");
    };
    assert_eq!(rhs.next_pipeline_op_id.get(), rhs.pipeline.len() as u32);
}

#[test]
fn chained_plan_refreshes_next_pipeline_op_id_after_chain_append() {
    let plan = plan_one("RETURN 1 AS n NEXT RETURN 2 AS n");

    assert_eq!(plan.next_pipeline_op_id.get(), plan.pipeline.len() as u32);
    let Some(PipelineOp::Chain(rhs)) = plan.pipeline.last() else {
        panic!("expected chain op");
    };
    assert_eq!(rhs.next_pipeline_op_id.get(), rhs.pipeline.len() as u32);
}

#[test]
fn anonymous_intermediate_node_does_not_leak_to_next_edge() {
    // Why: prior `leftmost_binding` fell back from `right_binding` to
    // `left_binding`, so `(a)-[]->()-[]->(c)` reported the second edge's left
    // endpoint as `Some(a)` instead of `None`, contradicting the parsed
    // topology.
    let plan = plan_one("MATCH (a)-[:K]->()-[:K]->(c) RETURN a, c");
    let pattern = plan.pattern_plan.as_ref().expect("pattern plan");
    let JoinTree::Expand {
        edge: outer_edge, ..
    } = &pattern.join_tree
    else {
        panic!("expected outer expand, got {:?}", pattern.join_tree);
    };
    assert!(outer_edge.right_binding.is_some());
    assert_eq!(
        outer_edge.left_binding, None,
        "left_binding of the second edge must be None for anonymous middle node"
    );
}

#[test]
fn scalar_functions_are_not_classified_as_aggregates_in_group_by() {
    // Why: prior `aggregate_name` returned Some for any single-segment
    // function call, so scalar functions like `length` were lifted into
    // `GroupBy.aggregates` even though they are pure scalars.
    let plan =
        plan_one("MATCH (n) RETURN length(n.name) AS l, count(*) AS c GROUP BY length(n.name)");
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
        !aggregate_names.iter().any(|name| name == "length"),
        "length must not be classified as an aggregate: {aggregate_names:?}"
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
        .filter_map(|column| column.name.map(|name| name.as_str().to_string()))
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
fn leading_optional_match_is_not_implemented() {
    let err = plan_err("OPTIONAL MATCH (a) RETURN a");
    assert!(matches!(
        err,
        PlannerError::NotImplemented {
            feature: "leading OPTIONAL MATCH (no preceding pipeline)",
            ..
        }
    ));
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
