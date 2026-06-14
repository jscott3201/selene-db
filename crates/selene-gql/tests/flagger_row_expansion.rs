//! Flagger coverage for row-expansion query forms.

use selene_core::{GraphId, feature_register::FeatureId};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, TxContext, analyze,
    execute_pattern, execute_pipeline, feature_walk, parse, plan,
};
use selene_graph::SharedGraph;

#[test]
fn for_list_feature_is_supported_and_recorded() {
    let source = "FOR x IN [1, 2] RETURN x";
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&FeatureId::GQ10),
        "{source} should record GQ10, observed {observed:?}"
    );
    assert_read_plan(source);
    assert_read_execution(source);
}

#[test]
fn non_iso_unwind_alias_is_rejected_at_parse_time() {
    let err = parse("UNWIND [1, 2] AS x RETURN x").expect_err("UNWIND is not ISO GQL");
    assert!(
        matches!(err, selene_gql::ParserError::SyntaxError { .. }),
        "expected syntax error, got {err:?}"
    );
    assert_eq!(err.gqlstatus().as_str(), "42001");
}

#[test]
fn for_position_features_are_supported_and_recorded() {
    for (source, expected) in [
        (
            "FOR x IN [1, 2] WITH ORDINALITY ord RETURN x, ord",
            FeatureId::GQ11,
        ),
        (
            "FOR x IN [1, 2] WITH OFFSET off RETURN x, off",
            FeatureId::GQ24,
        ),
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GQ10),
            "{source} should record base GQ10, observed {observed:?}"
        );
        assert!(
            observed.contains(&expected),
            "{source} should record {expected}, observed {observed:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

fn assert_read_plan(source: &str) {
    let _ = read_plan(source);
}

fn read_plan(source: &str) -> selene_gql::ExecutionPlan {
    let statement = parse(source).expect(source);
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).expect(source);
    plan(&analyzed, &EmptyProcedureRegistry).expect(source)
}

fn assert_read_execution(source: &str) {
    let plan = read_plan(source);
    let graph = SharedGraph::new(GraphId::new(9151));
    let mut ctx = TxContext::read_only(
        graph.read(),
        &plan.impl_defined_caps,
        &EmptyProcedureRegistry,
        graph.index_providers(),
    )
    .with_plan_metadata(&plan.expr_ids, &plan.subqueries);
    let input = if let Some(pattern) = &plan.pattern_plan {
        execute_pattern(pattern, &ctx).expect(source)
    } else {
        BindingTable::new(
            BindingTableSchema {
                columns: Vec::new(),
            },
            vec![Binding::empty()],
        )
    };
    execute_pipeline(&plan.pipeline, input, &mut ctx).expect(source);
}
