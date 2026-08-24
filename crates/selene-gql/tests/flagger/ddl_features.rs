use selene_gql::{GqlStatus, ParserError, SourceSpan, feature_walk, parse};
use selene_profile::{FeatureId, capability};

use super::assert_read_plan;

#[test]
fn mutation_feature_is_supported() {
    parse("MATCH (n) SET n.active = true RETURN n").expect("GD01 mutation syntax parses");
}

#[test]
fn open_graph_management_is_admitted_and_closed_forms_are_feature_rejected() {
    // GC04/GC05/GG01 are runtime-supported: the open graph type form parses
    // and stamps the ISO features. The closed-type forms are rejected with the
    // feature that owns the clause (ISO section 12.4 CR4-CR7).
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids("CREATE GRAPH demo ANY"),
        [FeatureId::GC04, FeatureId::GG01]
    );
    assert_eq!(
        ids("CREATE GRAPH IF NOT EXISTS demo ANY"),
        [FeatureId::GC04, FeatureId::GG01, FeatureId::GC05]
    );

    let source = "CREATE GRAPH /demo LIKE /source";
    let error = parse(source).expect_err(source);
    let ParserError::UnsupportedFeature {
        feature_id,
        display_name,
        span,
        hint,
    } = error
    else {
        panic!("expected UnsupportedFeature for {source:?}");
    };
    let record = capability(FeatureId::GG04).expect("GG04 capability");
    assert_eq!(feature_id, FeatureId::GG04);
    assert_eq!(display_name, record.name);
    assert_eq!(display_name, "Graph type like a graph");
    assert_eq!(span, SourceSpan::new(19, 12));
    assert_eq!(hint, record.non_support_rationale);

    for (source, expected) in [
        (
            "CREATE GRAPH demo ::{(City :City {name STRING})}",
            FeatureId::GG03,
        ),
        ("CREATE GRAPH demo ANY AS COPY OF source", FeatureId::GG05),
        (
            "CREATE GRAPH demo {(Person :Person {lastname STRING, joined DATE})} AS COPY OF source",
            FeatureId::GG05,
        ),
    ] {
        let error = selene_gql::parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        assert_feature(error, expected);
    }
    for source in [
        "CREATE GRAPH demo TYPED socialNetworkGraphType",
        "CREATE GRAPH demo ::socialNetworkGraphType",
    ] {
        let error = selene_gql::parse(source).expect_err(source);
        assert_eq!(error.gqlstatus().as_str(), "42N01");
        assert!(
            matches!(error, ParserError::NotImplemented { .. }),
            "{source}: expected NotImplemented, got {error:?}"
        );
    }
}

#[test]
fn drop_graph_stamps_gc04_and_the_bridge_extension() {
    // DROP GRAPH is ISO GC04 (+GC05 with IF EXISTS). IM_DROP_GRAPH stays
    // stamped while the compatibility session may execute the statement as
    // the bootstrap factory reset; M02-PR05 removes the bridge and the stamp.
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids("DROP GRAPH demo"),
        [FeatureId::GC04, FeatureId::IM_DROP_GRAPH]
    );
    assert_eq!(
        ids("DROP GRAPH IF EXISTS demo"),
        [FeatureId::GC04, FeatureId::GC05, FeatureId::IM_DROP_GRAPH]
    );
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
        "CREATE OR REPLACE GRAPH demo ANY",
        "CREATE OR REPLACE NODE TYPE :Person (name :: STRING)",
        "CREATE OR REPLACE EDGE TYPE :KNOWS (FROM :Person TO :Person)",
    ] {
        let error = selene_gql::parse(source).expect_err(source);
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
    parse("CALL pkg.fn(1)").expect("GP04 named CALL parses");
    parse("MATCH (n) CALL pkg.fn(n) RETURN n").expect("in-pipeline GP04 CALL parses");
}

fn assert_feature(error: ParserError, expected: FeatureId) {
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature, got {error:?}");
    };
    assert_eq!(feature_id, expected);
}
