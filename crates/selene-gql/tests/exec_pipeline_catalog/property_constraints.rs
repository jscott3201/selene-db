//! Catalog property constraint/default tests split out of the root pipeline catalog
//! binary to keep both files under the repository file-size cap.

use selene_gql::{
    CatalogOp, ExecutorError, GqlStatus, GqlType, PipelineOp, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, RecordType, SourceSpan,
};
use selene_graph::PropertyDefaultValue;

use super::{db_string, empty_closed_graph, planned, run_write};

fn list_default(items: Vec<PropertyDefaultValue>) -> PropertyDefaultValue {
    PropertyDefaultValue::List(items.into_iter().map(Box::new).collect())
}

#[test]
fn or_replace_catalog_ddl_is_deferred() {
    let graph = empty_closed_graph(3714);

    let mut or_replace = planned("CREATE NODE TYPE :Person ()");
    if let PipelineOp::Catalog(CatalogOp::CreateNodeType { or_replace, .. }) =
        &mut or_replace.pipeline[0]
    {
        *or_replace = true;
    }

    let err = run_write(&graph, &or_replace).expect_err("OR REPLACE is deferred");
    assert!(matches!(err, ExecutorError::ImplementationDefined { .. }));
}

#[test]
fn unique_property_constraint_is_deferred() {
    let graph = empty_closed_graph(3714);
    let plan = planned("CREATE NODE TYPE :Sensor (v :: STRING UNIQUE)");

    // UNIQUE is ISO-relevant but uniqueness enforcement is not yet implemented;
    // it surfaces as an honest 42N01 capability-gap deferral (not a generic
    // 5GQL0 internal error), mirroring the inline-INDEXED-on-edge deferral.
    let err = run_write(&graph, &plan).expect_err("UNIQUE property constraint is deferred");
    assert!(matches!(
        err,
        ExecutorError::FeatureNotSupportedYet {
            feature: "UNIQUE property constraint",
            ..
        }
    ));
    assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
}

#[test]
fn nothing_property_type_is_deferred() {
    let graph = empty_closed_graph(3715);
    let mut plan = planned("CREATE NODE TYPE :Person ()");
    let PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create node type");
    };
    properties.push(PlannedTypePropertyDef {
        name: db_string("payload"),
        gql_type: GqlType::Nothing,
        constraints: Vec::new(),
        span: SourceSpan::new(0, 1),
    });

    let err = run_write(&graph, &plan).expect_err("NOTHING type is deferred");

    assert!(matches!(
        err,
        ExecutorError::ImplementationDefined {
            detail: "type property GQL type not supported as property value type (Phase A)",
        }
    ));
}

#[test]
fn open_record_property_type_is_supported() {
    // A bare/open `RECORD` property lowers to a permissive RecordTyped declaration that
    // accepts any record value.
    let graph = empty_closed_graph(3716);
    let mut plan = planned("CREATE NODE TYPE :Person ()");
    let PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create node type");
    };
    properties.push(PlannedTypePropertyDef {
        name: db_string("payload"),
        gql_type: GqlType::Record(RecordType::Open),
        constraints: Vec::new(),
        span: SourceSpan::new(0, 1),
    });

    let (_table, outcome) = run_write(&graph, &plan).expect("open RECORD type executes");
    outcome.expect("open RECORD property type commits");
}

#[test]
fn default_property_constraint_accepts_supported_literal_ir() {
    let graph = empty_closed_graph(3716);
    let mut plan = planned("CREATE NODE TYPE :Person ()");
    let PipelineOp::Catalog(CatalogOp::CreateNodeType { properties, .. }) = &mut plan.pipeline[0]
    else {
        panic!("expected create node type");
    };
    let project = planned("RETURN 1 AS x")
        .pipeline
        .into_iter()
        .find_map(|op| match op {
            PipelineOp::Project(mut items) => items.pop(),
            _ => None,
        })
        .expect("project expr");
    properties.push(PlannedTypePropertyDef {
        name: db_string("age"),
        gql_type: GqlType::Integer,
        constraints: vec![PlannedTypePropertyConstraint::Default(
            project,
            SourceSpan::new(0, 1),
        )],
        span: SourceSpan::new(0, 1),
    });

    run_write(&graph, &plan)
        .expect("default constraint executes")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Integer(1))
    );
}

#[test]
fn bytes_default_property_constraint_accepts_byte_literal() {
    let graph = empty_closed_graph(3727);
    let plan = planned("CREATE NODE TYPE :Blob (payload :: BYTES DEFAULT X'CAFE')");

    run_write(&graph, &plan)
        .expect("bytes default constraint executes")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Bytes(vec![0xCA, 0xFE]))
    );
}

#[test]
fn uuid_default_property_constraint_accepts_uuid_literal() {
    let graph = empty_closed_graph(3728);
    let plan = planned(
        "CREATE NODE TYPE :Thing (id :: UUID DEFAULT UUID '018f1b6d-7b89-7cc0-9f40-2c6f8d4df101')",
    );

    run_write(&graph, &plan)
        .expect("UUID default constraint executes")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Uuid(db_string(
            "018f1b6d-7b89-7cc0-9f40-2c6f8d4df101"
        )))
    );
}

#[test]
fn json_default_property_constraint_accepts_json_string_literal() {
    let graph = empty_closed_graph(3729);
    let plan = planned(r#"CREATE NODE TYPE :Doc (payload :: JSON DEFAULT '{"b":2,"a":"don''t"}')"#);

    run_write(&graph, &plan)
        .expect("JSON default constraint executes")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Json(db_string(
            r#"{"a":"don't","b":2}"#
        )))
    );
}

#[test]
fn json_default_property_constraint_rejects_invalid_json_string() {
    let graph = empty_closed_graph(3730);
    let plan = planned("CREATE NODE TYPE :Doc (payload :: JSON DEFAULT 'not-json')");

    let err = run_write(&graph, &plan).expect_err("invalid JSON default rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_CHARACTER_VALUE_FOR_CAST);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("JSON DEFAULT string is not valid JSON")
    ));
}

#[test]
fn vector_default_property_constraint_accepts_numeric_list_literal() {
    let graph = empty_closed_graph(3738);
    let plan = planned("CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT [1.0, -0.0, 2])");

    run_write(&graph, &plan)
        .expect("VECTOR default constraint executes")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Vector(vec![
            1.0_f32.to_bits(),
            0.0_f32.to_bits(),
            2.0_f32.to_bits(),
        ]))
    );
}

#[test]
fn vector_default_property_constraint_rejects_empty_list() {
    let graph = empty_closed_graph(3739);
    let plan = planned("CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT [])");

    let err = run_write(&graph, &plan).expect_err("empty VECTOR default rejected");

    assert_eq!(err.gqlstatus().as_str(), "22G03");
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("VECTOR DEFAULT must be a non-empty")
    ));
}

#[test]
fn vector_default_property_constraint_rejects_non_numeric_element() {
    let graph = empty_closed_graph(3740);
    let plan = planned("CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT ['x'])");

    let err = run_write(&graph, &plan).expect_err("non-numeric VECTOR default rejected");

    assert_eq!(err.gqlstatus().as_str(), "22G03");
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("VECTOR DEFAULT list elements must be numeric literals")
    ));
}

#[test]
fn vector_default_property_constraint_rejects_out_of_range_component() {
    let graph = empty_closed_graph(3741);
    let plan = planned("CREATE NODE TYPE :Doc (embedding :: VECTOR DEFAULT [1.0e9999])");

    let err = run_write(&graph, &plan).expect_err("out-of-range VECTOR default rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::NUMERIC_VALUE_OUT_OF_RANGE);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("VECTOR DEFAULT component must be finite")
    ));
}

#[test]
fn list_default_property_constraint_accepts_typed_list_literals() {
    let graph = empty_closed_graph(3742);
    let plan = planned(
        r#"CREATE NODE TYPE :Doc (
            tags :: LIST<STRING> DEFAULT ['alpha', 'beta'],
            counts :: LIST<UINT64> DEFAULT [42],
            payloads :: LIST<JSON> DEFAULT ['{"b":2,"a":1}'],
            matrix :: LIST<LIST<INTEGER>> DEFAULT [[1, 2], [3]],
            embeddings :: LIST<VECTOR> DEFAULT [[1, 0], [0, 1]]
        )"#,
    );

    run_write(&graph, &plan)
        .expect("LIST defaults execute")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    let properties = &graph_type.node_types[0].properties;
    assert_eq!(
        properties[0].default,
        Some(list_default(vec![
            PropertyDefaultValue::String(db_string("alpha")),
            PropertyDefaultValue::String(db_string("beta")),
        ]))
    );
    assert_eq!(
        properties[1].default,
        Some(list_default(vec![PropertyDefaultValue::Uint(42)]))
    );
    assert_eq!(
        properties[2].default,
        Some(list_default(vec![PropertyDefaultValue::Json(db_string(
            r#"{"a":1,"b":2}"#
        ))]))
    );
    assert_eq!(
        properties[3].default,
        Some(list_default(vec![
            list_default(vec![
                PropertyDefaultValue::Integer(1),
                PropertyDefaultValue::Integer(2),
            ]),
            list_default(vec![PropertyDefaultValue::Integer(3)]),
        ]))
    );
    assert_eq!(
        properties[4].default,
        Some(list_default(vec![
            PropertyDefaultValue::Vector(vec![1.0_f32.to_bits(), 0.0_f32.to_bits()]),
            PropertyDefaultValue::Vector(vec![0.0_f32.to_bits(), 1.0_f32.to_bits()]),
        ]))
    );
}

#[test]
fn list_default_property_constraint_rejects_element_mismatch() {
    let graph = empty_closed_graph(3743);
    let plan = planned("CREATE NODE TYPE :Doc (scores :: LIST<INTEGER> DEFAULT ['x'])");

    let err = run_write(&graph, &plan).expect_err("LIST element mismatch rejected");

    assert_eq!(err.gqlstatus().as_str(), "22G03");
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("LIST DEFAULT element is not assignable")
    ));
}

#[test]
fn list_default_property_constraint_rejects_nested_shape_mismatch() {
    let graph = empty_closed_graph(3744);
    let plan = planned("CREATE NODE TYPE :Doc (matrix :: LIST<LIST<INTEGER>> DEFAULT [1])");

    let err = run_write(&graph, &plan).expect_err("nested LIST shape mismatch rejected");

    assert_eq!(err.gqlstatus().as_str(), "22G03");
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("nested LIST DEFAULT elements must be list literals")
    ));
}

#[test]
fn float_default_property_constraint_accepts_float_literal() {
    let graph = empty_closed_graph(3724);
    let plan = planned(
        "CREATE NODE TYPE :Metric (score :: FLOAT DEFAULT 1.5, small :: FLOAT32 DEFAULT 2.25)",
    );

    run_write(&graph, &plan)
        .expect("float defaults execute")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Float(1.5_f64.to_bits()))
    );
    assert_eq!(
        graph_type.node_types[0].properties[1].default,
        Some(PropertyDefaultValue::Float32(2.25_f32.to_bits()))
    );
}

#[test]
fn float_default_property_constraint_rejects_non_finite_literal() {
    let graph = empty_closed_graph(3731);
    let plan = planned("CREATE NODE TYPE :Metric (score :: FLOAT DEFAULT 1.0e9999)");

    let err = run_write(&graph, &plan).expect_err("non-finite float default rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::NUMERIC_VALUE_OUT_OF_RANGE);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("FLOAT DEFAULT literal must be finite")
    ));
}

#[test]
fn exact_numeric_default_property_constraint_accepts_coerced_literals() {
    let graph = empty_closed_graph(3733);
    let plan = planned(
        "CREATE NODE TYPE :Metric (u :: UINT64 DEFAULT 42, \
         i128 :: INT128 DEFAULT '-170141183460469231731687303715884105728', \
         u128 :: UINT128 DEFAULT '340282366920938463463374607431768211455', \
         dec_i :: DECIMAL DEFAULT 7, dec_s :: DECIMAL DEFAULT '123.450', \
         dec_f :: DECIMAL DEFAULT 1.25)",
    );

    run_write(&graph, &plan)
        .expect("exact numeric defaults execute")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Uint(42))
    );
    assert_eq!(
        graph_type.node_types[0].properties[1].default,
        Some(PropertyDefaultValue::Int128(i128::MIN))
    );
    assert_eq!(
        graph_type.node_types[0].properties[2].default,
        Some(PropertyDefaultValue::Uint128(u128::MAX))
    );
    assert_eq!(
        graph_type.node_types[0].properties[3].default,
        Some(PropertyDefaultValue::Decimal(db_string("7")))
    );
    assert_eq!(
        graph_type.node_types[0].properties[4].default,
        Some(PropertyDefaultValue::Decimal(db_string("123.450")))
    );
    assert_eq!(
        graph_type.node_types[0].properties[5].default,
        Some(PropertyDefaultValue::Decimal(db_string("1.25")))
    );
}

#[test]
fn exact_numeric_default_property_constraint_rejects_invalid_text() {
    let graph = empty_closed_graph(3734);
    let plan = planned("CREATE NODE TYPE :Metric (amount :: DECIMAL DEFAULT 'not-decimal')");

    let err = run_write(&graph, &plan).expect_err("invalid DECIMAL default rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::INVALID_CHARACTER_VALUE_FOR_CAST);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("not a valid DECIMAL")
    ));
}

#[test]
fn exact_numeric_default_property_constraint_rejects_range_overflow() {
    let graph = empty_closed_graph(3735);
    let plan = planned(
        "CREATE NODE TYPE :Metric (id :: UINT128 DEFAULT \
         '340282366920938463463374607431768211456')",
    );

    let err = run_write(&graph, &plan).expect_err("overflowing UINT128 default rejected");

    assert_eq!(err.gqlstatus(), GqlStatus::NUMERIC_VALUE_OUT_OF_RANGE);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("unsigned integer range")
    ));
}

#[test]
fn temporal_default_property_constraint_accepts_temporal_literals() {
    let graph = empty_closed_graph(3732);
    let plan = planned(
        "CREATE NODE TYPE :Event (d :: DATE DEFAULT DATE '2026-05-07', \
         ldt :: LOCAL DATETIME DEFAULT LOCAL DATETIME '2026-05-07T12:34:56', \
         zdt :: ZONED DATETIME DEFAULT ZONED DATETIME '2026-05-07T12:34:56-04:00', \
         lt :: LOCAL TIME DEFAULT LOCAL TIME '12:34:56', \
         zt :: ZONED TIME DEFAULT ZONED TIME '12:34:56-04:00', \
         dur :: DURATION DEFAULT DURATION 'PT1H2S')",
    );

    run_write(&graph, &plan)
        .expect("temporal defaults execute")
        .1
        .expect("commit succeeds");
    let graph_type = graph.graph_type().expect("closed graph type");
    assert_eq!(
        graph_type.node_types[0].properties[0].default,
        Some(PropertyDefaultValue::Date(db_string("2026-05-07")))
    );
    assert_eq!(
        graph_type.node_types[0].properties[1].default,
        Some(PropertyDefaultValue::LocalDateTime(db_string(
            "2026-05-07T12:34:56"
        )))
    );
    assert_eq!(
        graph_type.node_types[0].properties[2].default,
        Some(PropertyDefaultValue::ZonedDateTime(db_string(
            "2026-05-07T12:34:56-04"
        )))
    );
    assert_eq!(
        graph_type.node_types[0].properties[3].default,
        Some(PropertyDefaultValue::LocalTime(db_string("12:34:56")))
    );
    assert_eq!(
        graph_type.node_types[0].properties[4].default,
        Some(PropertyDefaultValue::ZonedTime(db_string("12:34:56-04")))
    );
    assert_eq!(
        graph_type.node_types[0].properties[5].default,
        Some(PropertyDefaultValue::Duration(db_string("PT1H2S")))
    );
}

#[test]
fn default_literal_must_match_declared_property_type() {
    let graph = empty_closed_graph(3725);
    let plan = planned("CREATE NODE TYPE :Person (active :: BOOLEAN DEFAULT 1)");

    let err = run_write(&graph, &plan).expect_err("default type mismatch");

    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("DEFAULT literal is not assignable")
                && message.contains("active")
    ));
}

#[test]
fn not_null_property_rejects_default_null() {
    let graph = empty_closed_graph(3726);
    let plan = planned("CREATE NODE TYPE :Person (active :: BOOLEAN NOT NULL DEFAULT NULL)");

    let err = run_write(&graph, &plan).expect_err("not null default null");

    assert_eq!(err.gqlstatus(), GqlStatus::DATATYPE_MISMATCH);
    assert!(matches!(
        err,
        ExecutorError::DataException { message, .. }
            if message.contains("NOT NULL property cannot default to NULL")
                && message.contains("active")
    ));
}
