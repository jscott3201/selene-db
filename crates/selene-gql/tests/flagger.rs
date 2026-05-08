//! Flagger feature-gating coverage.

use selene_core::feature_register::FeatureId;
use selene_gql::{ParserError, parse};

#[test]
fn union_feature_is_supported_and_otherwise_is_rejected() {
    parse("RETURN 1 UNION RETURN 2").expect("UNION is claimed");

    let error = parse("RETURN 1 OTHERWISE RETURN 2").expect_err("OTHERWISE is not claimed");
    assert_feature(error, FeatureId::GQ09);
}

#[test]
fn group_by_feature_is_supported() {
    parse("RETURN n GROUP BY n").expect("GROUP BY is claimed");
    parse("WITH n GROUP BY n RETURN n").expect("WITH GROUP BY is claimed");
}

#[test]
fn path_selector_features_are_supported_for_reachable_ast_variants() {
    for source in [
        "MATCH ALL (n) RETURN n",
        "MATCH ANY (n) RETURN n",
        "MATCH ALL SHORTEST (n)-[:K]->(m) RETURN m",
        "MATCH ANY SHORTEST (n)-[:K]->(m) RETURN m",
    ] {
        parse(source).expect(source);
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
fn mutation_feature_is_supported() {
    parse("MATCH (n) SET n.active = true RETURN n").expect("GD01 mutation is claimed");
}

#[test]
fn graph_management_and_if_exists_features_are_supported() {
    parse("CREATE GRAPH IF NOT EXISTS demo").expect("GC04 and GC03 are claimed");
    parse("DROP GRAPH IF EXISTS demo").expect("GC04 and GC03 are claimed");
}

#[test]
fn closed_type_ddl_features_are_supported() {
    parse("CREATE NODE TYPE IF NOT EXISTS :Person (name :: STRING)")
        .expect("GG02/GG20/GG21 are claimed");
    parse("DROP EDGE TYPE IF EXISTS :KNOWS").expect("type DROP is claimed");
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

fn assert_feature(error: ParserError, expected: FeatureId) {
    let ParserError::UnsupportedFeature { feature_id, .. } = error else {
        panic!("expected UnsupportedFeature, got {error:?}");
    };
    assert_eq!(feature_id, expected);
}
