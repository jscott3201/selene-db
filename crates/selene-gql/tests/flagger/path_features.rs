use selene_core::feature_register::FeatureId;
use selene_gql::{feature_walk, parse};

use super::{assert_read_execution, assert_read_plan};

#[test]
fn union_and_otherwise_features_are_supported() {
    parse("RETURN 1 UNION RETURN 2").expect("UNION is parser-compatibility accepted");
    assert_read_plan("RETURN 1 OTHERWISE RETURN 2");
}

#[test]
fn group_by_feature_is_supported() {
    parse("RETURN n GROUP BY n").expect("GROUP BY is parser-compatibility accepted");
    parse("WITH n GROUP BY n RETURN n").expect("WITH GROUP BY is parser-compatibility accepted");
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

    // Both counted forms must plan and execute, not merely be flagged.
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

    // ISO 39075:2024 §16.4 CR1/CR2: the two `<match mode>` features are
    // runtime-supported (G002 = DIFFERENT EDGES, G003 = REPEATABLE ELEMENTS).
    // Each parses cleanly and the flagger records its feature use without
    // rejecting it, then both plan and execute.
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
