//! Byte-string literal coverage for the native BYTES value type.

use std::sync::Arc;

use selene_core::{
    ByteStringType, DbString, GraphId, PropertyValueType, Record, Value,
    feature_register::FeatureId,
};
use selene_gql::{
    AnalyzedStatement, AnalyzedStatementKind, AnalyzedType, EmptyProcedureRegistry, GqlType,
    ImplDefinedCaps, ParserError, PipelineStatement, Session, StatementOutput,
    ast::{format_read_statement, structurally_eq},
    feature_walk, parse,
};
use selene_graph::{GraphTypeDef, PropertyElementType, RecordFieldType, SharedGraph};
use smallvec::smallvec;

fn first_value(source: &str) -> Value {
    let graph = SharedGraph::new(GraphId::new(14_200));
    let mut session = Session::new(&graph);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(14_201));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn first_value_with_caps(source: &str, caps: ImplDefinedCaps) -> Value {
    let graph = SharedGraph::new(GraphId::new(14_203));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_value_with_parameter(source: &str, name: &str, value: Value) -> Value {
    let graph = SharedGraph::new(GraphId::new(14_205));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string(name), value);
    let output = session
        .execute_source(source, &EmptyProcedureRegistry)
        .unwrap_or_else(|err| panic!("execute failed for `{source}`: {err:?}"));
    let StatementOutput::Rows(table) = output else {
        panic!("`{source}` produced non-row output");
    };
    table.rows()[0].values()[0].clone()
}

fn first_status_with_parameter(source: &str, name: &str, value: Value) -> String {
    let graph = SharedGraph::new(GraphId::new(14_206));
    let mut session = Session::new(&graph);
    session.bind_parameter(db_string(name), value);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn first_status_with_caps(source: &str, caps: ImplDefinedCaps) -> String {
    let graph = SharedGraph::new(GraphId::new(14_204));
    let mut session = Session::new(&graph).with_impl_defined_caps(caps);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .as_str()
        .to_owned()
}

fn bytes(values: &[u8]) -> Value {
    Value::Bytes(Arc::<[u8]>::from(values))
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn empty_closed_graph(id: u64) -> SharedGraph {
    SharedGraph::builder(GraphId::new(id))
        .bound_to(GraphTypeDef {
            name: db_string("byte.string.graph"),
            node_types: Vec::new(),
            edge_types: Vec::new(),
        })
        .unwrap()
        .build()
        .unwrap()
}

fn analyze_one(source: &str) -> AnalyzedStatement {
    let statement = parse(source).expect("test source parses");
    selene_gql::analyze(statement, &EmptyProcedureRegistry, None).expect("test source analyzes")
}

fn assert_feature_recorded(source: &str, expected: FeatureId) {
    let statement = parse(source).expect(source);
    let observed = feature_walk(&statement)
        .into_iter()
        .map(|feature| feature.feature_id)
        .collect::<Vec<_>>();
    assert!(
        observed.contains(&expected),
        "{source} should record {expected:?}, observed {observed:?}"
    );
}

fn projection_type(analyzed: &AnalyzedStatement, name: &str) -> AnalyzedType {
    let AnalyzedStatementKind::Query(query) = &analyzed.statement else {
        panic!("expected query statement");
    };
    let item = query
        .statements
        .iter()
        .filter_map(|statement| match statement {
            PipelineStatement::Return(clause) => Some(&clause.items),
            _ => None,
        })
        .flatten()
        .find(|item| {
            item.alias
                .clone()
                .is_some_and(|alias| alias.as_str() == name)
        })
        .unwrap_or_else(|| panic!("projection {name} exists"));
    let id = analyzed
        .expr_ids
        .get(&item.expr)
        .unwrap_or_else(|| panic!("projection {name} has an ExprId"));
    analyzed.expr_types.get(id).clone()
}

#[test]
fn byte_string_literal_executes_to_bytes_value() {
    assert_eq!(
        first_value("RETURN X'00 ff A5' AS payload"),
        bytes(&[0x00, 0xff, 0xa5])
    );
    assert_eq!(first_value("RETURN x'0a' AS payload"), bytes(&[0x0a]));
    assert_eq!(first_value("RETURN X'' AS payload"), bytes(&[]));
}

#[test]
fn byte_string_literal_chunks_concatenate_across_newline_separator() {
    assert_eq!(
        first_value("RETURN X'CA'\n'FE' AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn byte_string_literal_infers_bytes_and_is_typed_bytes() {
    let analyzed = analyze_one("RETURN X'CAFE' AS payload");
    assert_eq!(
        projection_type(&analyzed, "payload"),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BYTES AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn bounded_byte_string_forms_parse_flag_and_format() {
    for (source, expected) in [
        ("RETURN X'CAFE' IS TYPED BYTES(2) AS ok", FeatureId::GV37),
        ("RETURN X'CAFE' IS TYPED BYTES(1, 4) AS ok", FeatureId::GV36),
        ("RETURN X'CAFE' IS TYPED BINARY(2) AS ok", FeatureId::GV38),
        (
            "RETURN X'CAFE' IS TYPED VARBINARY(2) AS ok",
            FeatureId::GV37,
        ),
    ] {
        parse(source).unwrap_or_else(|err| panic!("bounded byte-string type parses: {err:?}"));
        assert_feature_recorded(source, FeatureId::GV35);
        assert_feature_recorded(source, expected);
    }

    let fixed = parse("RETURN X'CAFE' IS TYPED BINARY(2) AS ok").expect("source parses");
    let formatted = format_read_statement(&fixed).expect("formats");
    assert_eq!(formatted, "RETURN X'CAFE' IS TYPED BYTES(2, 2) AS ok");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&fixed, &reparsed));

    let variable = parse("RETURN X'CAFE' IS TYPED VARBINARY(4) AS ok").expect("source parses");
    let formatted = format_read_statement(&variable).expect("formats");
    assert_eq!(formatted, "RETURN X'CAFE' IS TYPED BYTES(4) AS ok");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&variable, &reparsed));
}

#[test]
fn bounded_byte_string_type_predicates_check_length_bounds() {
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BYTES(2) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED BYTES(2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CA' IS TYPED BYTES(2, 4) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED BYTES(2, 4) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BINARY(2) AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN X'CA' IS TYPED BINARY(2) AS ok"),
        Value::Bool(false)
    );
    assert_eq!(
        first_value("RETURN X'CAFE00' IS TYPED VARBINARY(2) AS ok"),
        Value::Bool(false)
    );
}

#[test]
fn bounded_byte_string_cast_pads_and_truncates_trailing_zeroes() {
    assert_eq!(
        first_value("RETURN CAST(X'CA' AS BYTES(2, 4)) AS payload"),
        bytes(&[0xca, 0x00])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CA' AS BINARY(3)) AS payload"),
        bytes(&[0xca, 0x00, 0x00])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE00' AS VARBINARY(2)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE00' AS BYTES(1, 2)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_status("RETURN CAST(X'CAFE01' AS VARBINARY(2)) AS payload"),
        "22001"
    );
}

#[test]
fn byte_string_literal_identity_cast_returns_bytes() {
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS BYTES) AS payload"),
        bytes(&[0xca, 0xfe])
    );
}

#[test]
fn byte_string_left_and_right_follow_iso_substring_rules() {
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', 2) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', 99) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE', 0) AS payload"),
        bytes(&[])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', CAST('2' AS INT128)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN right(X'CAFE00', CAST('2' AS UINT128)) AS payload"),
        bytes(&[0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE00', 2M) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN left(X'CAFE', null) AS payload"),
        Value::Null
    );
    assert_eq!(first_value("RETURN left(null, 1) AS payload"), Value::Null);
    assert_eq!(first_status("RETURN left(X'CAFE', -1) AS payload"), "22011");
    assert_eq!(
        first_status("RETURN left(X'CAFE', 1.5M) AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_follows_iso_trim_rules() {
    assert_eq!(
        first_value("RETURN TRIM(X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(LEADING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN TRIM(TRAILING X'00' FROM X'0000CAFE00') AS payload"),
        bytes(&[0x00, 0x00, 0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH FROM X'20CA20') AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH X'00' FROM null) AS payload"),
        Value::Null
    );
    assert_eq!(
        first_value("RETURN TRIM(BOTH null FROM X'0000') AS payload"),
        Value::Null
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'CAFE' FROM X'CA') AS payload"),
        "22027"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH 'x' FROM X'78') AS payload"),
        "22G03"
    );
    assert_eq!(
        first_status("RETURN TRIM(BOTH X'78' FROM 'xx') AS payload"),
        "22G03"
    );
}

#[test]
fn byte_string_trim_infers_bytes_and_records_gf07() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_feature_recorded(
        "RETURN TRIM(BOTH X'00' FROM X'00CA00') AS payload",
        FeatureId::GF07,
    );
    assert_feature_recorded("RETURN TRIM(X'20CA20') AS payload", FeatureId::GF07);
}

#[test]
fn bounded_byte_string_trim_and_concat_infer_plain_bytes() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN X'CA' || CAST(X'FE' AS BINARY(1)) AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CA' || CAST(X'FE' AS BINARY(1)) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        projection_type(
            &analyze_one("RETURN TRIM(BOTH CAST(X'00' AS BINARY(1)) FROM X'00CA00') AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
}

#[test]
fn byte_string_concatenation_uses_byte_string_result_type() {
    assert_eq!(
        first_value("RETURN X'CA' || X'FE00' AS payload"),
        bytes(&[0xca, 0xfe, 0x00])
    );
    assert_eq!(
        first_value("RETURN X'' || X'CA' AS payload"),
        bytes(&[0xca])
    );
    assert_eq!(first_status("RETURN X'CA' || 'FE' AS payload"), "22G03");
}

#[test]
fn byte_string_concatenation_truncates_only_zero_byte_overflow() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(3);
    assert_eq!(
        first_value_with_caps("RETURN X'CAFE' || X'0000' AS payload", caps),
        bytes(&[0xca, 0xfe, 0x00])
    );
}

#[test]
fn byte_string_concatenation_reports_right_truncation() {
    let caps = ImplDefinedCaps::default().with_max_byte_string_length(1);
    assert_eq!(
        first_status_with_caps("RETURN X'CA' || X'FE' AS payload", caps),
        "22001"
    );
}

#[test]
fn bare_byte_string_type_aliases_normalize_to_bytes() {
    assert_eq!(
        projection_type(
            &analyze_one("RETURN CAST(X'CAFE' AS BINARY) AS payload"),
            "payload"
        ),
        AnalyzedType::Resolved(GqlType::Bytes)
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED BINARY AS ok"),
        Value::Bool(true)
    );
    assert_eq!(
        first_value("RETURN CAST(X'CAFE' AS VARBINARY) AS payload"),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_value("RETURN X'CAFE' IS TYPED VARBINARY AS ok"),
        Value::Bool(true)
    );
}

#[test]
fn typed_parameters_enforce_bounded_byte_string_lengths() {
    assert_eq!(
        first_value_with_parameter(
            "RETURN $payload :: BYTES(2, 4) AS payload",
            "payload",
            bytes(&[0xca, 0xfe])
        ),
        bytes(&[0xca, 0xfe])
    );
    assert_eq!(
        first_status_with_parameter(
            "RETURN $payload :: BYTES(2, 4) AS payload",
            "payload",
            bytes(&[0xca])
        ),
        "22G03"
    );
}

#[test]
fn bounded_byte_string_type_bounds_reject_invalid_lengths() {
    for source in [
        "RETURN X'CA' IS TYPED BYTES(0) AS ok",
        "RETURN X'CA' IS TYPED BYTES(4, 2) AS ok",
        "RETURN X'CA' IS TYPED BINARY(0) AS ok",
        "RETURN X'CA' IS TYPED VARBINARY(0) AS ok",
    ] {
        parse(source).expect_err("invalid byte-string bounds reject");
    }
}

#[test]
fn byte_string_literal_formatter_canonicalizes_uppercase_hex() {
    let parsed = parse("RETURN x'00 ff a5' AS payload").expect("source parses");
    let formatted = format_read_statement(&parsed).expect("formats");
    assert_eq!(formatted, "RETURN X'00FFA5' AS payload");
    let reparsed = parse(&formatted).expect("formatted source reparses");
    assert!(structurally_eq(&parsed, &reparsed));
}

#[test]
fn byte_string_literal_rejects_odd_hex_digit_count() {
    let err = parse("RETURN X'0' AS payload").expect_err("odd hex count is rejected");
    let ParserError::SyntaxError { message, .. } = err else {
        panic!("expected syntax error");
    };
    assert!(message.contains("odd number of hexadecimal digits"));
}

#[test]
fn byte_string_literal_rejects_plain_space_between_chunks() {
    parse("RETURN X'CA' 'FE' AS payload").expect_err("chunk separator requires a newline");
}

#[test]
fn byte_string_literal_rejects_non_hex_digit() {
    parse("RETURN X'0G' AS payload").expect_err("non-hex digit is rejected");
}

#[test]
fn non_identity_bytes_casts_are_invalid_type_combinations() {
    for source in [
        "RETURN CAST('00' AS BYTES) AS payload",
        "RETURN CAST(1 AS BYTES) AS payload",
        "RETURN CAST(true AS BYTES) AS payload",
        "RETURN CAST(X'CAFE' AS STRING) AS payload",
        "RETURN CAST(X'CAFE' AS INTEGER) AS payload",
        "RETURN CAST(X'CAFE' AS BOOLEAN) AS payload",
        "RETURN CAST([X'CAFE'] AS LIST<STRING>) AS payload",
    ] {
        assert_eq!(first_status(source), "22G03", "{source}");
    }
}

#[test]
fn bounded_byte_string_catalog_preserves_descriptors_and_show() {
    let graph = empty_closed_graph(14_207);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Blob (\
                payload :: BYTES(2, 4) DEFAULT X'CAFE', \
                fixed :: BINARY(2), \
                chunks :: LIST<VARBINARY(2)> DEFAULT [X'01', X'0203'], \
                meta :: RECORD { digest :: BINARY(4), tag :: VARBINARY(1) } \
                    DEFAULT RECORD{digest: X'01020304', tag: X'AA'}\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");

    let graph_type = graph.graph_type().expect("graph type is bound");
    let properties = &graph_type.node_types[0].properties;
    assert_eq!(properties[0].value_type, PropertyValueType::Bytes);
    assert_eq!(properties[0].byte_string_type, ByteStringType::new(2, 4));
    assert_eq!(properties[1].byte_string_type, ByteStringType::new(2, 2));
    assert_eq!(
        properties[2].list_element_type,
        Some(PropertyElementType::ByteString(
            ByteStringType::new(0, 2).expect("valid byte-string type")
        ))
    );
    let record_fields = properties[3]
        .record_field_types
        .as_ref()
        .expect("record field descriptor");
    assert_eq!(
        record_fields.0[0].field_type,
        RecordFieldType::ByteString(ByteStringType::new(4, 4).expect("valid byte-string type"))
    );
    assert_eq!(
        record_fields.0[1].field_type,
        RecordFieldType::ByteString(ByteStringType::new(0, 1).expect("valid byte-string type"))
    );

    let show = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW succeeds");
    let StatementOutput::Rows(table) = show else {
        panic!("SHOW returns rows");
    };
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Blob (payload :: BYTES(2, 4) DEFAULT X'CAFE', fixed :: BYTES(2, 2), chunks :: LIST<BYTES(2)> DEFAULT [X'01', X'0203'], meta :: RECORD { digest :: BYTES(4, 4), tag :: BYTES(1) } DEFAULT RECORD{digest: X'01020304', tag: X'AA'})"
        ))
    );

    session
        .execute_source(
            "INSERT (:Blob { \
                payload: X'AABB', \
                fixed: X'0102', \
                chunks: [X'01', X'0203'], \
                meta: RECORD{digest: X'01020304', tag: X'AA'} \
            }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("valid bounded bytes insert succeeds");

    let err = session
        .execute_source(
            "INSERT (:Blob { \
                payload: X'AABBCCDDEE', \
                fixed: X'0102', \
                chunks: [X'01'], \
                meta: RECORD{digest: X'01020304', tag: X'AA'} \
            }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect_err("non-zero overflow violates BYTES(2,4)");
    assert_eq!(err.gqlstatus().as_str(), "22001");
    assert!(
        err.to_string().contains("payload"),
        "expected property-specific store-assignment error, got {err:?}"
    );
}

#[test]
fn bounded_byte_string_store_assignment_pads_and_truncates_zeroes() {
    let graph = empty_closed_graph(14_209);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Blob (\
                payload :: BYTES(2, 4), \
                fixed :: BINARY(2), \
                chunks :: LIST<VARBINARY(2)>, \
                meta :: RECORD { digest :: BINARY(4), tag :: VARBINARY(1) }\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("catalog DDL executes");
    session
        .execute_source(
            "INSERT (:Blob { \
                payload: X'AA', \
                fixed: X'01', \
                chunks: [X'0A', X'0B0000'], \
                meta: RECORD{digest: X'0102', tag: X'AA00'} \
            }) FINISH",
            &EmptyProcedureRegistry,
        )
        .expect("store assignment pads and truncates only zero bytes");

    let output = session
        .execute_source(
            "MATCH (b:Blob) RETURN b.payload AS payload, b.fixed AS fixed, \
             b.chunks AS chunks, b.meta AS meta",
            &EmptyProcedureRegistry,
        )
        .expect("match succeeds");
    let StatementOutput::Rows(table) = output else {
        panic!("MATCH returns rows");
    };
    assert_eq!(
        table.rows()[0].values(),
        &[
            bytes(&[0xAA, 0x00]),
            bytes(&[0x01, 0x00]),
            Value::List(vec![bytes(&[0x0A]), bytes(&[0x0B, 0x00])]),
            Value::Record(Box::new(Record::Open(smallvec![
                (db_string("digest"), bytes(&[0x01, 0x02, 0x00, 0x00])),
                (db_string("tag"), bytes(&[0xAA])),
            ]))),
        ]
    );
}

#[test]
fn bounded_byte_string_catalog_rejects_defaults_outside_descriptor() {
    for source in [
        "CREATE NODE TYPE :Blob (payload :: BYTES(2, 4) DEFAULT X'AA')",
        "CREATE NODE TYPE :Blob (chunks :: LIST<BYTES(2, 4)> DEFAULT [X'AA'])",
        "CREATE NODE TYPE :Blob (meta :: RECORD { payload :: BYTES(2, 4) } DEFAULT RECORD{payload: X'AA'})",
    ] {
        let graph = empty_closed_graph(14_208);
        let mut session = Session::new(&graph);
        let err = session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect_err("out-of-envelope byte default is rejected");
        assert!(
            err.to_string().contains("DEFAULT"),
            "expected DEFAULT validation error for `{source}`, got {err:?}"
        );
    }
}
