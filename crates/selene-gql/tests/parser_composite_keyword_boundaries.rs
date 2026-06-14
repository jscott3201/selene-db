//! Composite and chained query keyword-boundary regression coverage.

use selene_gql::{ParserError, Statement, parse};

fn assert_syntax_error(source: &str) {
    let error = parse(source).expect_err(source);
    assert!(
        matches!(error, ParserError::SyntaxError { .. }),
        "{source} must reject as syntax, got {error:?}"
    );
}

#[test]
fn composite_and_chained_keywords_require_boundaries() {
    for source in [
        "RETURN 1 AS n NEXTRETURN 2 AS n",
        "RETURN 1 AS n NEXTx RETURN 2 AS n",
        "RETURN 1 AS n UNIONALL RETURN 2 AS n",
        "RETURN 1 AS n UNIONx RETURN 2 AS n",
        "RETURN 1 AS n UNION ALLx RETURN 2 AS n",
        "RETURN 1 AS n UNION DISTINCTx RETURN 2 AS n",
        "RETURN 1 AS n INTERSECTALL RETURN 1 AS n",
        "RETURN 1 AS n INTERSECTx RETURN 1 AS n",
        "RETURN 1 AS n EXCEPTALL RETURN 1 AS n",
        "RETURN 1 AS n EXCEPTx RETURN 1 AS n",
        "RETURN 1 AS n OTHERWISEx RETURN 2 AS n",
        "CREATE SCHEMA /foo NEXTCREATE SCHEMA /bar",
        "CREATE SCHEMA /foo NEXTx CREATE SCHEMA /bar",
    ] {
        assert_syntax_error(source);
    }
}

#[test]
fn guarded_composite_and_chained_keywords_still_accept_iso_forms() {
    for source in [
        "RETURN 1 AS n NEXT RETURN 2 AS n",
        "RETURN 1 AS n UNION RETURN 2 AS n",
        "RETURN 1 AS n UNION ALL RETURN 2 AS n",
        "RETURN 1 AS n UNION DISTINCT RETURN 2 AS n",
        "RETURN 1 AS n INTERSECT RETURN 1 AS n",
        "RETURN 1 AS n INTERSECT ALL RETURN 1 AS n",
        "RETURN 1 AS n EXCEPT RETURN 1 AS n",
        "RETURN 1 AS n EXCEPT ALL RETURN 1 AS n",
        "RETURN 1 AS n OTHERWISE RETURN 2 AS n",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}

#[test]
fn guarded_chained_query_still_lowers_to_chained_statement() {
    let statement = parse("RETURN 1 AS n NEXT RETURN 2 AS n").expect("NEXT query parses");
    assert!(matches!(statement, Statement::Chained { .. }));
}
