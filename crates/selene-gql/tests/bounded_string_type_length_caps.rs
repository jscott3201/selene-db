//! Implementation-defined declared-length caps for bounded character-string
//! and byte-string type names.
//!
//! Fixed-length coercion pads values up to `min_len`, so declared
//! `CHAR(n)`/`BINARY(n)`-class lengths are allocation bounds for read-only
//! CAST evaluation, store assignment, and DEFAULT descriptors. These tests
//! pin the parse-time cap for every grammar form that accepts length bounds
//! and prove sub-cap funnel behavior is unchanged.

use std::sync::Arc;

use selene_core::{
    DbString, GraphId, MAX_BYTE_STRING_TYPE_LENGTH, MAX_CHARACTER_STRING_TYPE_LENGTH, Value,
};
use selene_gql::{
    EmptyProcedureRegistry, ImplDefinedCaps, ParserError, Session, StatementOutput, parse,
};
use selene_graph::{GraphTypeDef, SharedGraph};

const CHAR_CAP: u64 = MAX_CHARACTER_STRING_TYPE_LENGTH;
const BYTE_CAP: u64 = MAX_BYTE_STRING_TYPE_LENGTH;

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn bytes(values: &[u8]) -> Value {
    Value::Bytes(Arc::from(values))
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("bounded.length.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn first_value_in(session: &mut Session<'_>, source: &str) -> Value {
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(16_300));
    let mut session = Session::new(&graph);
    first_value_in(&mut session, source)
}

fn assert_syntax_reject(source: &str, needle: &str) {
    let err = parse(source).expect_err(source);
    let ParserError::SyntaxError { message, .. } = &err else {
        panic!("expected syntax error for `{source}`, got {err:?}");
    };
    assert!(
        message.contains(needle),
        "expected `{needle}` in parse error for `{source}`, got `{message}`"
    );
}

#[test]
fn declared_length_caps_are_pinned() {
    // The caps are durable-schema and allocation bounds; changing them is a
    // deliberate decision, not a drive-by edit.
    assert_eq!(MAX_CHARACTER_STRING_TYPE_LENGTH, 1 << 20);
    assert_eq!(MAX_BYTE_STRING_TYPE_LENGTH, 1 << 20);
}

#[test]
fn bounded_string_type_names_parse_at_the_declared_cap() {
    for source in [
        format!("RETURN 'a' IS TYPED STRING({CHAR_CAP}) AS ok"),
        format!("RETURN 'a' IS TYPED STRING(1, {CHAR_CAP}) AS ok"),
        format!("RETURN 'a' IS TYPED CHAR({CHAR_CAP}) AS ok"),
        format!("RETURN 'a' IS TYPED VARCHAR({CHAR_CAP}) AS ok"),
        format!("RETURN X'CA' IS TYPED BYTES({BYTE_CAP}) AS ok"),
        format!("RETURN X'CA' IS TYPED BYTES(1, {BYTE_CAP}) AS ok"),
        format!("RETURN X'CA' IS TYPED BINARY({BYTE_CAP}) AS ok"),
        format!("RETURN X'CA' IS TYPED VARBINARY({BYTE_CAP}) AS ok"),
    ] {
        parse(&source).unwrap_or_else(|err| panic!("`{source}` should parse at cap: {err:?}"));
    }
}

#[test]
fn bounded_string_type_names_reject_lengths_above_the_declared_cap() {
    let char_over = CHAR_CAP + 1;
    let byte_over = BYTE_CAP + 1;
    for (source, needle) in [
        (
            format!("RETURN 'a' IS TYPED STRING({char_over}) AS ok"),
            "character string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN 'a' IS TYPED STRING(1, {char_over}) AS ok"),
            "character string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN 'a' IS TYPED CHAR({char_over}) AS ok"),
            "character string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN 'a' IS TYPED VARCHAR({char_over}) AS ok"),
            "character string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN 'a' IS TYPED VARCHAR({}) AS ok", u64::MAX),
            "character string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN X'CA' IS TYPED BYTES({byte_over}) AS ok"),
            "byte string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN X'CA' IS TYPED BYTES(1, {byte_over}) AS ok"),
            "byte string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN X'CA' IS TYPED BINARY({byte_over}) AS ok"),
            "byte string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN X'CA' IS TYPED VARBINARY({byte_over}) AS ok"),
            "byte string length exceeds the implementation-defined maximum",
        ),
        (
            format!("RETURN X'CA' IS TYPED BINARY({}) AS ok", u64::MAX),
            "byte string length exceeds the implementation-defined maximum",
        ),
    ] {
        assert_syntax_reject(&source, needle);
    }
}

#[test]
fn allocation_sized_cast_targets_are_rejected_at_parse_time() {
    // The original resource-bound gap: a pure read-only CAST whose target
    // length sizes a multi-GiB padded allocation. Rejection must happen at
    // parse time so evaluation (and its Vec::with_capacity) is unreachable.
    assert_syntax_reject(
        "RETURN CAST(X'' AS BINARY(4000000000)) AS payload",
        "byte string length exceeds the implementation-defined maximum",
    );
    assert_syntax_reject(
        "RETURN CAST('a' AS CHAR(2000000000)) AS value",
        "character string length exceeds the implementation-defined maximum",
    );
    assert_syntax_reject(
        &format!("RETURN CAST(X'' AS VARBINARY({})) AS payload", u64::MAX),
        "byte string length exceeds the implementation-defined maximum",
    );
}

#[test]
fn bounded_string_type_names_reject_u64_overflow_lengths() {
    // 21 digits: above u64::MAX, so the length literal itself fails to parse
    // before any cap comparison.
    let overflow = "999999999999999999999";
    for (source, needle) in [
        (
            format!("RETURN 'a' IS TYPED STRING({overflow}) AS ok"),
            "character string length exceeds supported range",
        ),
        (
            format!("RETURN 'a' IS TYPED STRING(1, {overflow}) AS ok"),
            "character string length exceeds supported range",
        ),
        (
            format!("RETURN 'a' IS TYPED CHAR({overflow}) AS ok"),
            "character string length exceeds supported range",
        ),
        (
            format!("RETURN 'a' IS TYPED VARCHAR({overflow}) AS ok"),
            "character string length exceeds supported range",
        ),
        (
            format!("RETURN X'CA' IS TYPED BYTES({overflow}) AS ok"),
            "byte string length exceeds supported range",
        ),
        (
            format!("RETURN X'CA' IS TYPED BYTES(1, {overflow}) AS ok"),
            "byte string length exceeds supported range",
        ),
        (
            format!("RETURN X'CA' IS TYPED BINARY({overflow}) AS ok"),
            "byte string length exceeds supported range",
        ),
        (
            format!("RETURN X'CA' IS TYPED VARBINARY({overflow}) AS ok"),
            "byte string length exceeds supported range",
        ),
    ] {
        assert_syntax_reject(&source, needle);
    }
}

#[test]
fn catalog_ddl_rejects_lengths_above_the_declared_cap() {
    let char_over = CHAR_CAP + 1;
    let byte_over = BYTE_CAP + 1;
    for source in [
        format!("CREATE NODE TYPE :Doc (title :: STRING({char_over}))"),
        format!("CREATE NODE TYPE :Doc (title :: CHAR({char_over}))"),
        format!("CREATE NODE TYPE :Doc (title :: VARCHAR({char_over}))"),
        format!("CREATE NODE TYPE :Doc (payload :: BYTES({byte_over}))"),
        format!("CREATE NODE TYPE :Doc (payload :: BINARY({byte_over}))"),
        format!("CREATE NODE TYPE :Doc (payload :: VARBINARY({byte_over}))"),
        // Nested descriptor positions share the same type-name builder.
        format!("CREATE NODE TYPE :Doc (tags :: LIST<STRING({char_over})>)"),
        format!("CREATE NODE TYPE :Doc (meta :: RECORD {{ code :: CHAR({char_over}) }})"),
        format!("CREATE NODE TYPE :Doc (meta :: RECORD {{ raw :: BINARY({byte_over}) }})"),
    ] {
        let err = parse(&source).expect_err(&source);
        assert!(
            matches!(err, ParserError::SyntaxError { .. }),
            "expected syntax error for `{source}`, got {err:?}"
        );
    }
}

#[test]
fn cast_padding_below_the_cap_is_unchanged() {
    assert_eq!(
        first_value("RETURN CAST('a' AS CHAR(64)) AS value"),
        Value::String(db_string(&format!("a{}", " ".repeat(63))))
    );
    let mut padded = vec![0_u8; 64];
    padded[0] = 0xca;
    assert_eq!(
        first_value("RETURN CAST(X'CA' AS BINARY(64)) AS payload"),
        bytes(&padded)
    );
}

#[test]
fn cast_padding_at_the_cap_stays_executable() {
    // The cap is a bound, not a rejection of bounded work: a fixed-length
    // coercion at exactly the cap pads to cap characters and completes.
    let value = first_value(&format!("RETURN CAST('a' AS CHAR({CHAR_CAP})) AS value"));
    let Value::String(value) = value else {
        panic!("expected string value, got {value:?}");
    };
    assert_eq!(value.as_str().chars().count() as u64, CHAR_CAP);
    assert!(value.as_str().starts_with('a'));
}

#[test]
fn store_assignment_padding_below_the_cap_is_unchanged() {
    let graph = empty_closed_graph(16_301);
    let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);

    session
        .execute_source(
            "CREATE NODE TYPE :Doc (title :: CHAR(8), payload :: BINARY(4))",
            &EmptyProcedureRegistry,
        )
        .expect("catalog succeeds");
    session
        .execute_source(
            "INSERT (:Doc {title: 'a', payload: X'CA'}) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("insert pads fixed-length values");
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.title AS title"),
        Value::String(db_string("a       "))
    );
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.payload AS payload"),
        bytes(&[0xca, 0x00, 0x00, 0x00])
    );

    session
        .execute_source(
            "MATCH (n:Doc) SET n.title = 'xy', n.payload = X'CAFE' FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("SET applies store assignment");
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.title AS title"),
        Value::String(db_string("xy      "))
    );
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.payload AS payload"),
        bytes(&[0xca, 0xfe, 0x00, 0x00])
    );
}

#[test]
fn default_descriptors_below_the_cap_are_unchanged() {
    let graph = empty_closed_graph(16_302);
    let mut session = Session::new(&graph).with_impl_defined_caps(ImplDefinedCaps::DEFAULT);

    session
        .execute_source(
            "CREATE NODE TYPE :Doc (\
                title :: CHAR(8) DEFAULT 'a', \
                payload :: BINARY(4) DEFAULT X'CA'\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("DEFAULT descriptor coercion succeeds");
    session
        .execute_source("INSERT (:Doc) FINISH", &EmptyProcedureRegistry)
        .expect("insert materializes padded defaults");
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.title AS title"),
        Value::String(db_string("a       "))
    );
    assert_eq!(
        first_value_in(&mut session, "MATCH (n:Doc) RETURN n.payload AS payload"),
        bytes(&[0xca, 0x00, 0x00, 0x00])
    );
}
