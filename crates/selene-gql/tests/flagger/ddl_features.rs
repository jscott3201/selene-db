use selene_core::feature_register::FeatureId;
use selene_gql::{GqlStatus, ParserError, feature_walk, parse};

use super::assert_read_plan;

#[test]
fn mutation_feature_is_supported() {
    parse("MATCH (n) SET n.active = true RETURN n")
        .expect("GD01 mutation syntax is parser-compatibility accepted");
}

#[test]
fn create_graph_is_rejected_before_planning() {
    // CREATE GRAPH stays GC04-rejected under D1 single-graph: the engine cannot
    // create a second graph. DROP GRAPH is split out (now IM_DROP_GRAPH).
    for source in [
        "CREATE GRAPH demo",
        "CREATE GRAPH IF NOT EXISTS demo",
        "CREATE GRAPH demo ANY",
        "CREATE GRAPH demo TYPED socialNetworkGraphType",
        "CREATE GRAPH demo ::socialNetworkGraphType",
        "CREATE GRAPH demo ::{(City :City {name STRING})}",
        "CREATE GRAPH /demo LIKE /source",
        "CREATE GRAPH demo ANY AS COPY OF source",
        "CREATE GRAPH demo {(Person :Person {lastname STRING, joined DATE})} AS COPY OF source",
    ] {
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
fn deferred_grammar_surfaces_report_not_implemented_with_42n01() {
    // PARSE-17: `RETURN NO BINDINGS` and `SELECT FROM <graph-name>` parse at
    // the grammar level but are not available as user GQL. Pin the exact
    // NotImplemented variant + the 42N01 (FEATURE_NOT_SUPPORTED) status.
    for source in ["MATCH (n) RETURN NO BINDINGS", "SELECT * FROM my_graph"] {
        let error = parse(source).expect_err(source);
        assert!(
            matches!(error, ParserError::NotImplemented { .. }),
            "expected NotImplemented for {source:?}, got {error:?}"
        );
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::FEATURE_NOT_SUPPORTED,
            "{source:?} must report 42N01"
        );
        assert_eq!(error.gqlstatus().as_str(), "42N01", "{source:?}");
    }
}

#[test]
fn non_iso_list_iteration_expressions_are_syntax_errors() {
    for source in [
        "RETURN [x IN [1, 2, 3] WHERE x > 1 | x]",
        "RETURN ALL(x IN [1, 2, 3] WHERE x > 0)",
        "RETURN ANY(x IN [1, 2, 3] WHERE x = 2)",
        "RETURN NONE(x IN [1, 2, 3] WHERE x = 4)",
        "RETURN SINGLE(x IN [1, 2, 3] WHERE x = 2)",
        "RETURN REDUCE(acc = 0, x IN [1, 2, 3] | acc + x)",
    ] {
        let error = parse(source).expect_err(source);
        assert!(
            matches!(error, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {error:?}"
        );
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::SYNTAX_ERROR,
            "{source:?} must report 42001"
        );
    }
}

#[test]
fn closed_type_ddl_syntax_is_observed() {
    // The parser still observes GG02/GG20/GG21 syntax after runtime-support
    // withdrawal. Bare `:Name` forms do not flag GG21; only the explicit `=>`
    // form does. This test asserts parse acceptance, not support or claim state.
    parse("CREATE NODE TYPE IF NOT EXISTS :Person (name :: STRING)")
        .expect("closed type syntax parses");
    parse("DROP EDGE TYPE IF EXISTS :KNOWS").expect("type DROP syntax parses");
}

#[test]
fn alter_edge_type_stamps_implementation_defined_feature() {
    let statement =
        parse("ALTER EDGE TYPE :KNOWS (FROM :Person, :Org TO :Person, since :: STRING)")
            .expect("ALTER EDGE TYPE parses");
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        observed.contains(&FeatureId::IM_ALTER_EDGE_TYPE),
        "ALTER EDGE TYPE must flag the implementation-defined extension; observed {observed:?}"
    );
    assert!(
        observed.contains(&FeatureId::GG02) && observed.contains(&FeatureId::GG20),
        "ALTER EDGE TYPE remains closed-type DDL; observed {observed:?}"
    );
}

#[test]
fn alter_node_type_stamps_only_its_implementation_defined_type_features() {
    let statement = parse("ALTER NODE TYPE :Person (active :: BOOLEAN DEFAULT true)")
        .expect("ALTER NODE TYPE parses");
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();

    assert!(
        observed.contains(&FeatureId::IM_ALTER_NODE_TYPE),
        "ALTER NODE TYPE must flag its implementation-defined extension; observed {observed:?}"
    );
    assert!(
        observed.contains(&FeatureId::GG02) && observed.contains(&FeatureId::GG20),
        "ALTER NODE TYPE remains closed-type DDL; observed {observed:?}"
    );
    assert!(
        !observed.contains(&FeatureId::GG21),
        "the bare ALTER type name does not write an explicit key label set; observed {observed:?}"
    );
}

#[test]
fn bare_type_ddl_flags_gg02_gg20_but_not_gg21() {
    // 813: type DDL flags GG02 (closed graph type) + GG20 (explicit element type
    // names — the `:Name` after NODE/EDGE TYPE is an explicit `<node/edge type
    // name>`, ISO §18.2/18.3). GG21 "Explicit element type key label sets"
    // requires the explicit `<...type key label set>` (`[ <label set phrase> ]
    // <implies>`, the `=>` marker); the bare `:Name` form leaves the key label
    // set *implied* per §18.2 SR5c, so GG21 must NOT flag here.
    for source in [
        "CREATE NODE TYPE :Person (name :: STRING)",
        "CREATE EDGE TYPE :KNOWS (since :: INTEGER)",
        "DROP NODE TYPE :Person",
        "SHOW NODE TYPES",
    ] {
        let statement = parse(source).unwrap_or_else(|err| panic!("{source} parses: {err:?}"));
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GG02) && observed.contains(&FeatureId::GG20),
            "{source}: type DDL must flag GG02 + GG20; observed {observed:?}"
        );
        assert!(
            !observed.contains(&FeatureId::GG21),
            "{source}: bare `:Name` form has no explicit key label set; GG21 must NOT flag; observed {observed:?}"
        );
    }
}

#[test]
fn explicit_key_label_set_flags_gg21() {
    // 813: an explicit `<...type key label set>` (the `=>` <implies> marker, ISO
    // §18.2/18.3) flags GG21 in addition to GG02 + GG20. Covers both the node
    // and edge surface and the property-types-content form (`:Person => (...)`).
    for source in [
        "CREATE NODE TYPE :Person => (name :: STRING)",
        "CREATE EDGE TYPE :KNOWS => (since :: INTEGER)",
        "CREATE NODE TYPE :Account => ()",
    ] {
        let statement = parse(source).unwrap_or_else(|err| panic!("{source} parses: {err:?}"));
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GG21),
            "{source}: explicit key label set (`=>`) must flag GG21; observed {observed:?}"
        );
        assert!(
            observed.contains(&FeatureId::GG02) && observed.contains(&FeatureId::GG20),
            "{source}: GG21 still flags GG02 + GG20; observed {observed:?}"
        );
    }
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
    // The existing type-DDL flags are unchanged on every path: GG02 (closed
    // graph type) + GG20 (explicit element type names). GG21 ("Explicit element
    // type key label sets") flags ONLY when the source writes the explicit
    // `<...type key label set>` (`=>`, ISO §18.2/18.3); a DROP statement carries
    // no key label set, so GG21 must NOT be flagged here.
    for statement in [&cascade_node, &restrict, &default] {
        let observed = ids(statement);
        assert!(observed.contains(&FeatureId::GG02));
        assert!(observed.contains(&FeatureId::GG20));
        assert!(
            !observed.contains(&FeatureId::GG21),
            "DROP type DDL has no explicit key label set; GG21 must NOT be flagged; observed {observed:?}"
        );
    }
}

#[test]
fn named_procedure_call_feature_is_supported() {
    parse("CALL pkg.fn(1)").expect("GP04 named CALL is parser-compatibility accepted");
    parse("MATCH (n) CALL pkg.fn(n) RETURN n")
        .expect("in-pipeline GP04 CALL is parser-compatibility accepted");
}

fn assert_feature(error: ParserError, expected: FeatureId) {
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature, got {error:?}");
    };
    assert_eq!(feature_id, expected);
}
