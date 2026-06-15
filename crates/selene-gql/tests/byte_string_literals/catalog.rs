use super::*;

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
fn bounded_byte_string_defaults_are_store_assigned() {
    let graph = empty_closed_graph(14_208);
    let mut session = Session::new(&graph);
    session
        .execute_source(
            "CREATE NODE TYPE :Blob (\
                payload :: BYTES(2, 4) DEFAULT X'AA', \
                fixed :: BINARY(2) DEFAULT X'01', \
                chunks :: LIST<BINARY(2)> DEFAULT [X'0A', X'0B0000'], \
                meta :: RECORD { digest :: BINARY(4), tag :: VARBINARY(1) } \
                    DEFAULT RECORD{digest: X'0102', tag: X'AA00'}\
            )",
            &EmptyProcedureRegistry,
        )
        .expect("DEFAULT descriptor coercion succeeds");

    let show = session
        .execute_source("SHOW NODE TYPES", &EmptyProcedureRegistry)
        .expect("SHOW succeeds");
    let StatementOutput::Rows(table) = show else {
        panic!("SHOW returns rows");
    };
    assert_eq!(
        table.rows()[0].values()[1],
        Value::String(db_string(
            "CREATE NODE TYPE :Blob (payload :: BYTES(2, 4) DEFAULT X'AA00', fixed :: BYTES(2, 2) DEFAULT X'0100', chunks :: LIST<BYTES(2, 2)> DEFAULT [X'0A00', X'0B00'], meta :: RECORD { digest :: BYTES(4, 4), tag :: BYTES(1) } DEFAULT RECORD{digest: X'01020000', tag: X'AA'})"
        ))
    );

    session
        .execute_source("INSERT (:Blob) FINISH", &EmptyProcedureRegistry)
        .expect("insert materializes coerced defaults");
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
            Value::List(vec![bytes(&[0x0A, 0x00]), bytes(&[0x0B, 0x00])]),
            Value::Record(Box::new(Record::Open(smallvec![
                (db_string("digest"), bytes(&[0x01, 0x02, 0x00, 0x00])),
                (db_string("tag"), bytes(&[0xAA])),
            ]))),
        ]
    );
}

#[test]
fn bounded_byte_string_defaults_reject_non_zero_truncation() {
    for source in [
        "CREATE NODE TYPE :Blob (payload :: BYTES(2, 4) DEFAULT X'AABBCCDDEE')",
        "CREATE NODE TYPE :Blob (chunks :: LIST<BYTES(2, 4)> DEFAULT [X'AABBCCDDEE'])",
        "CREATE NODE TYPE :Blob (meta :: RECORD { payload :: BYTES(2, 4) } DEFAULT RECORD{payload: X'AABBCCDDEE'})",
    ] {
        let graph = empty_closed_graph(14_210);
        let mut session = Session::new(&graph);
        let err = session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect_err("non-zero DEFAULT overflow is rejected");
        assert_eq!(err.gqlstatus().as_str(), "22001");
        assert!(
            err.to_string().contains("DEFAULT"),
            "expected DEFAULT validation error for `{source}`, got {err:?}"
        );
    }
}
