use super::*;

#[test]
fn malformed_inputs_return_syntax_error() {
    for source in ["RETURN", "RETURN 1 AS", "RTRN 1", ""] {
        assert!(matches!(
            parse(source),
            Err(ParserError::SyntaxError { .. })
        ));
    }
}

#[test]
fn signed_integer_overflow_reports_syntax_error() {
    // `-9223372036854775808` is i64::MIN; pest produces unary(-) over the
    // unsigned magnitude `9223372036854775808`, which doesn't fit in i64.
    // Signed numeric literals are parsed as unary expressions; reject a
    // bare magnitude that overflows i64 as a syntax error.
    let err = parse("RETURN -9223372036854775808").expect_err("magnitude overflow should error");
    assert!(matches!(err, ParserError::SyntaxError { .. }));
}

#[test]
fn malformed_underscores_in_integer_rejected() {
    for source in ["RETURN 1__2", "RETURN 1_"] {
        let err = parse(source).expect_err("malformed underscores should error");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected SyntaxError for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn parse_rejects_non_count_aggregate_star_shapes() {
    for source in [
        "RETURN sum(*)",
        "RETURN count(DISTINCT *)",
        "RETURN avg(*)",
        "RETURN collect_list(*)",
    ] {
        let err = parse(source).expect_err("invalid aggregate star shape should reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn parse_rejects_zero_argument_aggregates() {
    for source in [
        "RETURN count()",
        "RETURN sum()",
        "RETURN collect_list(DISTINCT)",
    ] {
        let err = parse(source).expect_err("aggregate without value expression should reject");
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn parse_rejects_non_iso_list_subscript_operator() {
    let err =
        parse("RETURN [10, 20, 30][1] AS value").expect_err("list subscript is not ISO GQL syntax");
    assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR);
}

#[test]
fn non_iso_sql_drift_predicates_are_syntax_errors() {
    // `LIKE` and `BETWEEN` are SQL drift with native ISO replacements
    // (STARTS WITH / ENDS WITH / CONTAINS and `x >= lo AND x <= hi`); the
    // grammar must reject them outright rather than accept-then-flag.
    for source in [
        "RETURN n.name LIKE 'a%'",
        "RETURN n.name NOT LIKE 'a%'",
        "RETURN n.age BETWEEN 1 AND 3",
        "RETURN n.age NOT BETWEEN 1 AND 3",
    ] {
        let err = parse(source).expect_err(source);
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn non_iso_modulo_and_temporal_and_sql_comment_are_syntax_errors() {
    // `%` infix modulo (use ISO `MOD(x, y)`), `.prop AT TIME ...` temporal
    // access, and the SQL `--` line comment are all non-ISO and removed.
    for source in [
        "RETURN 5 % 2",
        "RETURN n.created AT TIME 'UTC'",
        "RETURN 1 -- trailing comment",
    ] {
        let err = parse(source).expect_err(source);
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }

    // The ISO replacements and remaining comment forms still parse.
    assert!(matches!(
        only_item("RETURN MOD(5, 2) AS m").expr,
        ValueExpr::FunctionCall { .. }
    ));
    parse("RETURN 1 // trailing comment").expect("// line comment still parses");
    parse("RETURN 1 /* block comment */ AS x").expect("/* */ block comment still parses");
}

#[test]
fn quantifier_range_with_max_below_min_rejected() {
    for source in [
        "MATCH (a)-[*5..2]-(b) RETURN a",
        "MATCH (a)-[*{5,2}]-(b) RETURN a",
    ] {
        let err = parse(source).expect_err("max < min should error");
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected syntax error for {source:?}, got {err:?}"
        );
    }
}

#[test]
fn conflicting_quantifiers_in_edge_pattern_rejected() {
    // edge_interior accepts a quantifier and the outer edge accepts one
    // too. Specifying both must error rather than letting the second
    // silently overwrite the first.
    let err = parse("MATCH (a)-[r*1..2*3..4]->(b) RETURN a")
        .expect_err("conflicting quantifiers should error");
    assert!(
        matches!(err, ParserError::SyntaxError { .. }),
        "expected syntax error, got {err:?}"
    );
}
