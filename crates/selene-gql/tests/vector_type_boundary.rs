//! Regression coverage for vector value support versus ISO GQL type names.

use selene_gql::{GqlStatus, parse};

#[test]
fn vector_is_not_a_user_spelled_type_name() {
    for source in [
        "RETURN CAST(1 AS VECTOR) AS value",
        "RETURN 1 IS TYPED VECTOR AS ok",
        "RETURN $value :: VECTOR AS value",
        "CREATE NODE TYPE :Embedding (embedding :: VECTOR)",
        "CREATE NODE TYPE :Embedding (embedding :: LIST<VECTOR>)",
        "CREATE NODE TYPE :Embedding (metadata :: RECORD{embedding :: VECTOR})",
    ] {
        let err = parse(source).unwrap_err();
        assert_eq!(err.gqlstatus(), GqlStatus::SYNTAX_ERROR, "{source}");
    }
}

#[test]
fn vector_remains_available_as_an_identifier() {
    parse("MATCH (vector) RETURN vector").expect("vector parses as an identifier");
}
