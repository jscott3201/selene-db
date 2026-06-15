use selene_core::feature_register::FeatureId;
use selene_gql::{GqlStatus, ParserError, feature_walk, parse};

use super::{assert_read_execution, assert_read_plan};

#[test]
fn no_escape_character_string_literals_flag_gl11() {
    let ids = |source: &str| {
        feature_walk(&parse(source).expect(source))
            .into_iter()
            .map(|feature| feature.feature_id)
            .collect::<Vec<_>>()
    };

    assert!(
        !ids("RETURN 'literal' AS v").contains(&FeatureId::GL11),
        "ordinary escaped strings must not flag GL11"
    );

    for source in [
        r"RETURN @'literal' AS v",
        "RETURN @`literal` AS v",
        "RETURN DATE @'2026-05-07' AS v",
        "RETURN DURATION @'PT1S' AS v",
        "RETURN UUID @'550e8400-e29b-41d4-a716-446655440000' AS v",
        "RETURN PROPERTY_EXISTS(n, @'name') AS v",
        "SESSION SET TIME ZONE @'UTC'",
    ] {
        let observed = ids(source);
        assert!(
            observed.contains(&FeatureId::GL11),
            "{source} must flag GL11; observed {observed:?}"
        );
    }

    let uuid = ids("RETURN UUID @'550e8400-e29b-41d4-a716-446655440000' AS v");
    assert!(
        uuid.contains(&FeatureId::IM_UUID),
        "UUID no-escape literal must retain IM_UUID; observed {uuid:?}"
    );

    let duration = ids("RETURN DURATION @'PT1S' AS v");
    assert!(
        duration.contains(&FeatureId::GV41),
        "DURATION no-escape literal must retain GV41; observed {duration:?}"
    );

    let property_exists = ids("RETURN PROPERTY_EXISTS(n, @'name') AS v");
    assert!(
        property_exists.contains(&FeatureId::G115),
        "PROPERTY_EXISTS no-escape key must retain G115; observed {property_exists:?}"
    );

    let time_zone = ids("SESSION SET TIME ZONE @'UTC'");
    assert!(
        time_zone.contains(&FeatureId::GS15),
        "SESSION SET TIME ZONE no-escape literal must retain GS15; observed {time_zone:?}"
    );
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
        "FOR x IN [1, 2] RETURN stddev_pop(x)",
        "FOR x IN [1, 2] RETURN stddev_samp(x)",
        "FOR x IN [1, 2] RETURN collect_list(x)",
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
        "FOR x IN [1, 2] RETURN collect(x)",
        "FOR x IN [1, 2] RETURN average(x)",
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
fn list_subscript_is_rejected_before_feature_flagging() {
    let err =
        parse("RETURN [10, 20, 30][1] AS first").expect_err("list subscript is not ISO GQL syntax");
    assert!(matches!(err, ParserError::SyntaxError { .. }));
    assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);

    let bare_list = parse("RETURN [10, 20, 30] AS items").expect("bare list parses");
    let bare_ids = feature_walk(&bare_list)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        bare_ids.contains(&FeatureId::GV50),
        "bare list literal must still flag GV50"
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
