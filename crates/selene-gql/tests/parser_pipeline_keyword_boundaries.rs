//! Pipeline keyword boundary regression coverage.

use selene_gql::parse;

#[test]
fn pipeline_statement_keywords_require_boundaries() {
    for source in [
        "LETx = 1 RETURN x",
        "FORx IN [1] RETURN x",
        "FOR x INDEX [1] RETURN x",
        "FOR x IN [1] WITHORDINALITY ord RETURN x",
        "FOR x IN [1] WITHOFFSET off RETURN x",
        "WITHx AS x RETURN x",
    ] {
        assert!(
            parse(source).is_err(),
            "{source} must reject keyword prefix"
        );
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
    ] {
        parse(source).unwrap_or_else(|error| panic!("{source} should parse: {error:?}"));
    }
}
