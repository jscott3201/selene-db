//! Flagger feature-gating coverage.

use selene_core::{GraphId, feature_register::FeatureId};
use selene_gql::{
    Binding, BindingTable, BindingTableSchema, EmptyProcedureRegistry, GqlStatus, ParserError,
    TxContext, analyze, execute_pattern, execute_pipeline, feature_walk, parse, plan,
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
fn counted_shortest_selectors_flag_g019_and_g020() {
    // ISO §16.6 CR10/11: SHORTEST N PATHS is the counted shortest path search
    // (G019); SHORTEST [N] GROUP[S] is the counted shortest group search (G020).
    // SHORTEST GROUP / SHORTEST GROUPS default N=1 and still flag G020.
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    let counted_path = ids("MATCH SHORTEST 3 (n)-[:K]->(m) RETURN m");
    assert!(
        counted_path.contains(&FeatureId::G019),
        "SHORTEST 3 must flag G019; observed {counted_path:?}"
    );
    assert!(
        !counted_path.contains(&FeatureId::G020),
        "SHORTEST 3 (no GROUP) must NOT flag G020; observed {counted_path:?}"
    );

    for source in [
        "MATCH SHORTEST 2 GROUPS (n)-[:K]->(m) RETURN m",
        "MATCH SHORTEST GROUP (n)-[:K]->(m) RETURN m",
        "MATCH SHORTEST GROUPS (n)-[:K]->(m) RETURN m",
    ] {
        let observed = ids(source);
        assert!(
            observed.contains(&FeatureId::G020),
            "{source} must flag G020; observed {observed:?}"
        );
        assert!(
            !observed.contains(&FeatureId::G019),
            "{source} (GROUP form) must NOT flag G019; observed {observed:?}"
        );
    }

    // Both counted forms must plan and execute (claimed, not merely flagged).
    for source in [
        "MATCH SHORTEST 3 (n)-[:K]->(m) RETURN m",
        "MATCH SHORTEST 2 GROUPS (n)-[:K]->(m) RETURN m",
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
fn quantifier_and_match_mode_features_are_recorded() {
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

    // ISO 39075:2024 §16.4 CR1/CR2: the two `<match mode>` features are claimed
    // (G002 = DIFFERENT EDGES, G003 = REPEATABLE ELEMENTS). Each parses cleanly
    // and the flagger RECORDS its feature-use (a capability claim, not a
    // rejection trigger), then both plan and execute.
    let different =
        feature_walk(&parse("MATCH DIFFERENT EDGES (n) RETURN n").expect("G002 parses"))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
    assert!(
        different.contains(&FeatureId::G002),
        "DIFFERENT EDGES must record G002; observed {different:?}"
    );
    let repeatable =
        feature_walk(&parse("MATCH REPEATABLE ELEMENTS (n) RETURN n").expect("G003 parses"))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
    assert!(
        repeatable.contains(&FeatureId::G003),
        "REPEATABLE ELEMENTS must record G003; observed {repeatable:?}"
    );

    for source in [
        "MATCH DIFFERENT EDGES (n) RETURN n",
        "MATCH REPEATABLE ELEMENTS (n) RETURN n",
    ] {
        assert_read_plan(source);
        assert_read_execution(source);
    }
}

#[test]
fn match_mode_keywords_tolerate_whitespace_and_comments() {
    // ISO §16.4 grammar `^"DIFFERENT" ~ ^"EDGES"` (and the REPEATABLE form) skip
    // implicit WHITESPACE *and* COMMENTs between the two keywords, so every
    // separator spelling is a legal G002/G003 form. `build_match_mode` must
    // accept them — it dispatches on the leading keyword token, not a
    // single-space string compare against the raw span.
    let records = |source: &str| {
        feature_walk(&parse(source).unwrap_or_else(|e| panic!("{source:?} must parse: {e:?}")))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };
    for source in [
        "MATCH DIFFERENT  EDGES (n) RETURN n",        // two spaces
        "MATCH DIFFERENT\nEDGES (n) RETURN n",        // newline
        "MATCH DIFFERENT\tEDGES (n) RETURN n",        // tab
        "MATCH DIFFERENT /* c */ EDGES (n) RETURN n", // block comment
    ] {
        assert!(
            records(source).contains(&FeatureId::G002),
            "DIFFERENT EDGES with a non-single-space separator must record G002: {source:?}"
        );
    }
    for source in [
        "MATCH REPEATABLE  ELEMENTS (n) RETURN n",
        "MATCH REPEATABLE\nELEMENTS (n) RETURN n",
        "MATCH REPEATABLE // c\nELEMENTS (n) RETURN n", // line comment
    ] {
        assert!(
            records(source).contains(&FeatureId::G003),
            "REPEATABLE ELEMENTS with a non-single-space separator must record G003: {source:?}"
        );
    }
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
fn datetime_value_functions_record_temporal_features() {
    for (source, expected) in [
        ("RETURN CURRENT_DATE", FeatureId::GV39),
        ("RETURN DATE()", FeatureId::GV39),
        ("RETURN TIME()", FeatureId::GV39),
        ("RETURN DATETIME()", FeatureId::GV39),
        ("RETURN LOCAL_TIME", FeatureId::GV39),
        ("RETURN LOCAL_TIME()", FeatureId::GV39),
        ("RETURN LOCAL_DATETIME()", FeatureId::GV39),
        ("RETURN CURRENT_TIME", FeatureId::GV40),
        ("RETURN ZONED_TIME()", FeatureId::GV40),
        ("RETURN CURRENT_TIMESTAMP", FeatureId::GV40),
        ("RETURN ZONED_DATETIME()", FeatureId::GV40),
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

    for source in [
        "UNWIND [1, 2] AS x RETURN collect(x)",
        "UNWIND [1, 2] AS x RETURN average(x)",
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            !observed.contains(&FeatureId::GF10),
            "non-ISO aggregate alias must stay unattributed, observed {observed:?}"
        );
    }
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
fn closed_type_ddl_features_are_supported() {
    // GG02 (closed graph type) + GG20 (explicit element type names) are claimed,
    // as is GG21 (explicit element type key label sets, 813) — but the bare
    // `:Name` forms below DON'T flag GG21 (only the explicit `=>` form does;
    // see `explicit_key_label_set_flags_gg21`). This test only asserts parse
    // acceptance of the implicit forms.
    parse("CREATE NODE TYPE IF NOT EXISTS :Person (name :: STRING)")
        .expect("GG02/GG20 are claimed");
    parse("DROP EDGE TYPE IF EXISTS :KNOWS").expect("type DROP is claimed");
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
fn list_subscript_stamps_im_list_subscript_but_bare_list_does_not() {
    // `list[i]` is the selene-db vendor subscript (no ISO operator exists), so it
    // must flag IM_LIST_SUBSCRIPT on every use; a bare list literal flags only
    // GV50 and never the subscript.
    let subscript = parse("RETURN [10, 20, 30][1] AS first").expect("subscript parses");
    let bare_list = parse("RETURN [10, 20, 30] AS items").expect("bare list parses");

    let ids = |statement| {
        feature_walk(statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    let subscript_ids = ids(&subscript);
    assert!(
        subscript_ids.contains(&FeatureId::IM_LIST_SUBSCRIPT),
        "list subscript must flag IM_LIST_SUBSCRIPT"
    );
    assert!(
        subscript_ids.contains(&FeatureId::GV50),
        "the subscripted list literal still flags GV50"
    );
    assert!(
        !ids(&bare_list).contains(&FeatureId::IM_LIST_SUBSCRIPT),
        "a bare list literal must NOT flag IM_LIST_SUBSCRIPT"
    );
}

#[test]
fn record_value_form_flags_gv45_while_type_spellings_flag_gv46_47_48() {
    // The `RECORD{..}` VALUE constructor is GV45 (ISO §20.18 <record constructor>) and is
    // distinct from the record TYPE spellings: open `RECORD` is GV47, closed `RECORD{..}`
    // is GV46, nested is GV48. Pins that the record surface is actually flagged per clause
    // 24.6 (not merely listed in SUPPORTED_FEATURES) and that the value form is GV45 — not
    // GV47, which belongs to the open record *type*.
    let ids = |statement| {
        feature_walk(statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    let value = parse("RETURN RECORD {x: 1} AS r").expect("record value parses");
    let value_ids = ids(&value);
    assert!(
        value_ids.contains(&FeatureId::GV45),
        "a RECORD value constructor must flag GV45; observed {value_ids:?}"
    );
    assert!(
        !value_ids.contains(&FeatureId::GV47),
        "the value form must NOT flag GV47 (that is the open record TYPE); observed {value_ids:?}"
    );

    let open =
        parse("RETURN RECORD {name: 'Ada'} IS TYPED RECORD AS t").expect("open record type parses");
    let open_ids = ids(&open);
    assert!(
        open_ids.contains(&FeatureId::GV47),
        "an open RECORD type must flag GV47; observed {open_ids:?}"
    );

    let nested = parse(
        "RETURN RECORD {inner: RECORD {flag: true}} IS TYPED RECORD{inner :: RECORD{flag :: BOOL}} AS t",
    )
    .expect("nested closed record type parses");
    let nested_ids = ids(&nested);
    assert!(
        nested_ids.contains(&FeatureId::GV46) && nested_ids.contains(&FeatureId::GV48),
        "a nested closed RECORD type must flag GV46 + GV48; observed {nested_ids:?}"
    );
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
    parse("CALL pkg.fn(1)").expect("GP04 named CALL is claimed");
    parse("MATCH (n) CALL pkg.fn(n) RETURN n").expect("in-pipeline GP04 CALL is claimed");
}

#[test]
fn byte_string_length_type_forms_record_specific_gv_features() {
    for (source, expected, name) in [
        (
            "RETURN n IS TYPED BYTES(1, 16)",
            FeatureId::GV36,
            "Specified byte string minimum length",
        ),
        (
            "RETURN n IS TYPED BYTES(16)",
            FeatureId::GV37,
            "Specified byte string maximum length",
        ),
        (
            "RETURN n IS TYPED BINARY(16)",
            FeatureId::GV38,
            "Specified byte string fixed length",
        ),
    ] {
        let statement = parse(source).expect(source);
        let observed = feature_walk(&statement)
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>();
        assert!(
            observed.contains(&FeatureId::GV35),
            "{source} should record GV35"
        );
        assert!(
            observed.contains(&expected),
            "{source} should record {name} ({expected:?}), observed {observed:?}"
        );
    }
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
