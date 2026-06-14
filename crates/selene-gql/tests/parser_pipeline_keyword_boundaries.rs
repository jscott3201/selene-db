//! Pipeline keyword boundary regression coverage.

use selene_gql::{ast::format_read_statement, parse};

#[test]
fn pipeline_statement_keywords_require_boundaries() {
    for source in [
        "LETx = 1 RETURN x",
        "FORx IN [1] RETURN x",
        "FOR x INDEX [1] RETURN x",
        "FOR x IN [1] WITHORDINALITY ord RETURN x",
        "FOR x IN [1] WITHOFFSET off RETURN x",
        "WITHx AS x RETURN x",
        "FILTERWHERE true RETURN true",
        "FILTERx RETURN x",
        "RETURN 1 AS x ORDERBY x",
        "RETURN 1 AS x ORDER BY x DESCNULLS FIRST",
        "RETURN 1 AS x ORDER BY x NULLSFIRST",
        "OFFSET1 RETURN 1",
        "SKIP1 RETURN 1",
        "LIMIT1 RETURN 1",
        "RETURNx AS x",
        "RETURN NO BINDINGSx",
        "RETURN 1 ASx",
        "RETURN 1 AS x GROUPBY x",
        "RETURN 1 AS x HAVINGtrue",
        "MATCH (n) WHEREtrue RETURN n",
        "MATCH (n WHEREtrue) RETURN n",
        "CALL foo() YIELD bar ASbaz RETURN baz",
        "RETURN CAST(1 ASINT8) AS x",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject keyword prefix"
        );
    }
}

#[test]
fn guarded_modifier_prefixes_stay_identifiers() {
    for source in ["RETURN DISTINCTx AS x", "RETURN NOBINDINGS AS x"] {
        let statement =
            parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
        let formatted = format_read_statement(&statement)
            .unwrap_or_else(|error| panic!("{source} should format: {error:?}"));
        assert_eq!(formatted, source);
    }
}

#[test]
fn guarded_pipeline_keywords_still_accept_iso_forms() {
    for source in [
        "LET x = 1 RETURN x",
        "LET VALUE x INT8 = 1 RETURN x",
        "FOR x IN [1] RETURN x",
        "FOR x IN [1] WITH ORDINALITY ord RETURN x, ord",
        "FOR x IN [1] WITH OFFSET off RETURN x, off",
        "WITH 1 AS x RETURN x",
        "FILTER WHERE true RETURN true",
        "FILTER true RETURN true",
        "RETURN 1 AS x ORDER BY x DESC NULLS FIRST",
        "RETURN 1 AS x ORDER BY x ASC NULLS LAST",
        "OFFSET 1 RETURN 1",
        "SKIP 1 RETURN 1",
        "LIMIT 1 RETURN 1",
        "RETURN DISTINCT 1 AS x",
        "RETURN 1 AS x GROUP BY x",
        "RETURN 1 AS x HAVING true",
        "MATCH (n) WHERE true RETURN n",
        "MATCH (n WHERE true) RETURN n",
        "CALL foo() YIELD bar AS baz RETURN baz",
        "RETURN CAST(1 AS INT8) AS x",
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}
