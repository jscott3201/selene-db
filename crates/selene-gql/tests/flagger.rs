//! Flagger feature-gating coverage.

use selene_core::{GraphId, feature_register::FeatureId};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, ParserError, TxContext,
    analyze, execute_pattern, execute_pipeline, feature_walk, parse, plan,
};
use selene_graph::SharedGraph;

#[test]
fn union_and_otherwise_features_are_supported() {
    parse("RETURN 1 UNION RETURN 2").expect("UNION is claimed");
    assert_read_plan("RETURN 1 OTHERWISE RETURN 2");
}

#[test]
fn group_by_feature_is_supported() {
    parse("RETURN n GROUP BY n").expect("GROUP BY is claimed");
    parse("WITH n GROUP BY n RETURN n").expect("WITH GROUP BY is claimed");
}

#[test]
fn path_selector_features_are_supported() {
    for source in [
        "MATCH ALL (n) RETURN n",
        "MATCH ANY (n) RETURN n",
        "MATCH ALL SHORTEST (n)-[:K]->(m) RETURN m",
        "MATCH ANY SHORTEST (n)-[:K]->(m) RETURN m",
    ] {
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

#[test]
fn path_mode_features_are_supported_and_recorded() {
    for (source, expected) in [
        ("MATCH WALK (n) RETURN n", FeatureId::G010),
        ("MATCH TRAIL (n) RETURN n", FeatureId::G011),
        ("MATCH SIMPLE (n) RETURN n", FeatureId::G012),
        ("MATCH ACYCLIC (n) RETURN n", FeatureId::G013),
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&expected),
            "{source} should record {expected}, observed {observed:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

#[test]
fn quantifier_features_are_recorded_and_match_modes_rejected() {
    let bounded = parse("MATCH (a)-[r:K*1..2]->(b) RETURN r").expect("bounded parses");
    let unbounded =
        parse("MATCH TRAIL (a)-[r:K+]->(b)-[q?]->(c) RETURN r, q").expect("unbounded parses");
    let bounded_features = feature_walk(&bounded)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    let unbounded_features = feature_walk(&unbounded)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    for expected in [FeatureId::G036, FeatureId::G060] {
        assert!(
            bounded_features.contains(&expected),
            "bounded quantifier should record {expected}, observed {bounded_features:?}"
        );
    }
    for expected in [FeatureId::G036, FeatureId::G037, FeatureId::G061] {
        assert!(
            unbounded_features.contains(&expected),
            "unbounded/questioned quantifier should record {expected}, observed {unbounded_features:?}"
        );
    }

    assert_feature(
        parse("MATCH DIFFERENT EDGES (n) RETURN n").expect_err("G002 unsupported"),
        FeatureId::G002,
    );
    assert_feature(
        parse("MATCH REPEATABLE ELEMENTS (n) RETURN n").expect_err("G003 unsupported"),
        FeatureId::G003,
    );
}

#[test]
fn is_predicate_feature_family_is_supported() {
    for source in [
        "RETURN n IS DIRECTED",
        "RETURN n IS LABELED :Person",
        "RETURN n IS SOURCE OF e",
        "RETURN n IS DESTINATION OF e",
    ] {
        parse(source).expect(source);
    }
}

#[test]
fn graph_predicate_functions_are_supported() {
    for source in [
        "RETURN ALL_DIFFERENT(a, b)",
        "RETURN SAME(a, b)",
        "RETURN PROPERTY_EXISTS(n, 'name')",
    ] {
        parse(source).expect(source);
    }
}

#[test]
fn gf01_numeric_functions_are_supported_and_recorded() {
    for source in [
        "RETURN abs(-3)",
        "RETURN mod(7, 4)",
        "RETURN floor(1.8)",
        "RETURN ceil(1.2)",
        "RETURN ceiling(1.2)",
        "RETURN sqrt(9)",
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GF01),
            "{source} should record GF01, observed {observed:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

#[test]
fn element_id_function_is_supported_and_recorded() {
    let source = "MATCH (n) RETURN element_id(n)";
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        observed.contains(&FeatureId::G100),
        "{source} should record G100, observed {observed:?}"
    );
    assert_read_plan(source);
    assert_read_execution(source);
}

#[test]
fn cardinality_function_is_supported_and_recorded() {
    let source = "RETURN cardinality([1, 2, 3])";
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        observed.contains(&FeatureId::GF12),
        "{source} should record GF12, observed {observed:?}"
    );
    assert_read_plan(source);
    assert_read_execution(source);
}

#[test]
fn gf10_iso_aggregate_functions_are_recorded_without_collect_alias() {
    for source in [
        "UNWIND [1, 2] AS x RETURN stddev_pop(x)",
        "UNWIND [1, 2] AS x RETURN stddev_samp(x)",
        "UNWIND [1, 2] AS x RETURN collect_list(x)",
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GF10),
            "{source} should record GF10, observed {observed:?}"
        );
        assert_read_plan(source);
        assert_read_execution(source);
    }

    let collect_alias = parse("UNWIND [1, 2] AS x RETURN collect(x)").expect("collect parses");
    let observed = feature_walk(&collect_alias)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        !observed.contains(&FeatureId::GF10),
        "non-ISO collect alias must stay unattributed, observed {observed:?}"
    );
}

#[test]
fn mutation_feature_is_supported() {
    parse("MATCH (n) SET n.active = true RETURN n").expect("GD01 mutation is claimed");
}

#[test]
fn create_graph_is_rejected_before_planning() {
    // CREATE GRAPH stays GC04-rejected under D1 single-graph: the engine cannot
    // create a second graph. DROP GRAPH is split out (now IM_DROP_GRAPH).
    for source in ["CREATE GRAPH demo", "CREATE GRAPH IF NOT EXISTS demo"] {
        let error = parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        assert_feature(error, FeatureId::GC04);
    }
}

#[test]
fn drop_graph_stamps_im_drop_graph_and_parses() {
    // BRIEF-152 / audit Item 10: DROP GRAPH ships as the IM_DROP_GRAPH
    // factory-reset extension (a supported vendor flag), so it parses through to
    // the executor instead of dying in the flagger like CREATE GRAPH. IF EXISTS
    // is informational under D1 and adds no extra flag.
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };
    for source in ["DROP GRAPH demo", "DROP GRAPH IF EXISTS demo"] {
        let observed = ids(source);
        assert!(
            observed.contains(&FeatureId::IM_DROP_GRAPH),
            "{source} must flag IM_DROP_GRAPH"
        );
        assert!(
            !observed.contains(&FeatureId::GC04),
            "{source} must NOT flag GC04 (that stays CREATE GRAPH only)"
        );
    }
}

#[test]
fn intersect_and_except_composite_set_ops_are_supported() {
    for source in [
        "RETURN 1 INTERSECT RETURN 2",
        "RETURN 1 INTERSECT ALL RETURN 2",
        "RETURN 1 EXCEPT RETURN 2",
        "RETURN 1 EXCEPT ALL RETURN 2",
    ] {
        assert_read_plan(source);
    }
}

#[test]
fn or_replace_catalog_ddl_is_not_implemented() {
    for source in [
        "CREATE OR REPLACE GRAPH demo",
        "CREATE OR REPLACE NODE TYPE :Person (name :: STRING)",
        "CREATE OR REPLACE EDGE TYPE :KNOWS (FROM :Person TO :Person)",
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            matches!(error, ParserError::NotImplemented { .. }),
            "expected NotImplemented for {source:?}, got {error:?}"
        );
    }
}

#[test]
fn closed_type_ddl_features_are_supported() {
    parse("CREATE NODE TYPE IF NOT EXISTS :Person (name :: STRING)")
        .expect("GG02/GG20/GG21 are claimed");
    parse("DROP EDGE TYPE IF EXISTS :KNOWS").expect("type DROP is claimed");
}

#[test]
fn drop_cascade_stamps_im_drop_cascade_but_restrict_and_default_do_not() {
    // CASCADE is the IM_DROP_CASCADE vendor extension — it must flag on every
    // use. RESTRICT and the default carry only the existing type-DDL flags.
    let cascade_node = parse("DROP NODE TYPE :Sensor CASCADE").expect("CASCADE node parses");
    let cascade_edge = parse("DROP EDGE TYPE :KNOWS CASCADE").expect("CASCADE edge parses");
    let restrict = parse("DROP NODE TYPE :Sensor RESTRICT").expect("RESTRICT parses");
    let default = parse("DROP NODE TYPE :Sensor").expect("default parses");

    let ids = |statement| {
        feature_walk(statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    assert!(
        ids(&cascade_node).contains(&FeatureId::IM_DROP_CASCADE),
        "CASCADE node drop must flag IM_DROP_CASCADE"
    );
    assert!(
        ids(&cascade_edge).contains(&FeatureId::IM_DROP_CASCADE),
        "CASCADE edge drop must flag IM_DROP_CASCADE"
    );
    assert!(
        !ids(&restrict).contains(&FeatureId::IM_DROP_CASCADE),
        "explicit RESTRICT must NOT flag IM_DROP_CASCADE"
    );
    assert!(
        !ids(&default).contains(&FeatureId::IM_DROP_CASCADE),
        "default drop must NOT flag IM_DROP_CASCADE"
    );
    // The existing type-DDL flags are unchanged on every path.
    for statement in [&cascade_node, &restrict, &default] {
        let observed = ids(statement);
        assert!(observed.contains(&FeatureId::GG02));
        assert!(observed.contains(&FeatureId::GG20));
        assert!(observed.contains(&FeatureId::GG21));
    }
}

#[test]
fn named_procedure_call_feature_is_supported() {
    parse("CALL pkg.fn(1)").expect("GP04 named CALL is claimed");
    parse("MATCH (n) CALL pkg.fn(n) RETURN n").expect("in-pipeline GP04 CALL is claimed");
}

#[test]
fn real_type_spelling_is_rejected_before_canonicalization() {
    let error = parse("RETURN n IS TYPED REAL").expect_err("REAL spelling is unclaimed");
    let ParserError::UnsupportedFeature {
        feature_id,
        display_name,
        ..
    } = error
    else {
        panic!("expected UnsupportedFeature");
    };
    assert_eq!(feature_id, FeatureId::GV20);
    assert_eq!(display_name, "Approximate value type: REAL");
}

#[test]
fn normalized_predicate_has_no_feature_id_and_stays_unflagged() {
    parse("RETURN n IS NORMALIZED").expect("NORMALIZED has no feature ID");
}

#[test]
fn typed_parameter_feature_is_recorded_for_value_and_limit_surfaces() {
    let typed_value = feature_walk(&parse("RETURN $id :: INT").expect("source parses"))
        .into_iter()
        .filter(|feature| feature.feature_id == FeatureId::IM_TYPED_PARAMS)
        .count();
    assert_eq!(typed_value, 1);

    let typed_limit =
        feature_walk(&parse("MATCH (n) RETURN n LIMIT $count :: INT").expect("source parses"))
            .into_iter()
            .filter(|feature| feature.feature_id == FeatureId::IM_TYPED_PARAMS)
            .count();
    assert_eq!(typed_limit, 1);

    let bare_value = feature_walk(&parse("RETURN $id").expect("source parses"))
        .into_iter()
        .any(|feature| feature.feature_id == FeatureId::IM_TYPED_PARAMS);
    assert!(!bare_value);
}

fn assert_feature(error: ParserError, expected: FeatureId) {
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature, got {error:?}");
    };
    assert_eq!(feature_id, expected);
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
