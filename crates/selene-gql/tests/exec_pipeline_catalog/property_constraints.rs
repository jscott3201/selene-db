//! Catalog property constraint/default tests split out of the root pipeline catalog
//! binary to keep both files under the repository file-size cap.

use selene_gql::{
    CatalogOp, ExecutorError, GqlStatus, GqlType, PipelineOp, PlannedTypePropertyConstraint,
    PlannedTypePropertyDef, RecordType, SourceSpan,
};
use selene_graph::PropertyDefaultValue;

use super::{db_string, empty_closed_graph, planned, run_write};

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
fn json_default_property_constraint_accepts_json_string_literal() {
    let graph = empty_closed_graph(3728);
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
    let graph = empty_closed_graph(3729);
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
fn unsupported_default_literal_returns_feature_not_supported() {
    let graph = empty_closed_graph(3724);
    let plan = planned("CREATE NODE TYPE :Metric (score :: FLOAT DEFAULT 1.5)");

    let err = run_write(&graph, &plan).expect_err("float default unsupported");

    assert_eq!(err.gqlstatus(), GqlStatus::FEATURE_NOT_SUPPORTED);
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
