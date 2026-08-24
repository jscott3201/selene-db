//! Explicit facade-request runtime contract coverage.

use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc};

use selene_core::{DbString, GraphId, Value, db_string};
use selene_gql::{
    BuiltinProcedureRegistry, CallPlanCache, CatalogSessionOutput, EmptyProcedureRegistry, GqlType,
    RequestExecutionInput, RequestParameter, Session, StatementOutput, analyze, is_parameter_name,
    parse,
};
use selene_graph::SharedGraph;

fn parameter_name(name: &str) -> DbString {
    db_string(name).unwrap()
}

fn input(
    parameters: impl IntoIterator<Item = (&'static str, RequestParameter)>,
    timestamp: jiff::Timestamp,
    time_zone: jiff::tz::TimeZone,
) -> RequestExecutionInput {
    RequestExecutionInput::new(
        parameters
            .into_iter()
            .map(|(name, parameter)| (parameter_name(name), parameter))
            .collect::<BTreeMap<_, _>>(),
        timestamp,
        time_zone,
    )
}

fn execute(
    graph: &SharedGraph,
    source: &str,
    request: RequestExecutionInput,
) -> Result<CatalogSessionOutput, selene_gql::ExecutorError> {
    Session::new(graph).execute_source_catalog_request(source, &EmptyProcedureRegistry, request)
}

fn values(output: CatalogSessionOutput) -> Vec<Value> {
    let CatalogSessionOutput::Statement(StatementOutput::Rows(table)) = output else {
        panic!("expected row output");
    };
    assert_eq!(table.row_count(), 1);
    table.rows()[0].values().to_vec()
}

#[test]
fn parameter_name_helper_is_equivalent_to_source_grammar() {
    for name in ["value", "_2", "RETURN", "Δείγμα", "变量2"] {
        assert!(is_parameter_name(name), "{name:?}");
        parse(&format!("RETURN ${name}")).expect("accepted helper name parses after $");
    }
    for name in ["", "$value", "2value", "has-dash", "with space"] {
        assert!(!is_parameter_name(name), "{name:?}");
        if let Ok(statement) = parse(&format!("RETURN ${name}")) {
            let analyzed = analyze(statement, &EmptyProcedureRegistry, None);
            assert!(
                analyzed.is_err()
                    || analyzed
                        .unwrap()
                        .parameters
                        .iter()
                        .all(|parameter| parameter.name.as_str() != name),
                "no successful parse can decode the full invalid spelling {name:?}"
            );
        }
    }
}

#[test]
fn analyzer_contract_records_spans_and_inherits_source_declarations() {
    let source = "RETURN $x AS before, $x::INTEGER AS declared, $other";
    let statement = parse(source).unwrap();
    let analyzed = analyze(statement, &EmptyProcedureRegistry, None).unwrap();

    assert_eq!(analyzed.parameters.len(), 3);
    assert!(
        analyzed
            .parameters
            .windows(2)
            .all(|pair| pair[0].span.byte_offset <= pair[1].span.byte_offset)
    );
    let x = analyzed
        .parameters
        .iter()
        .filter(|parameter| parameter.name.as_str() == "x")
        .collect::<Vec<_>>();
    assert_eq!(x.len(), 2);
    assert!(
        x.iter()
            .all(|parameter| parameter.declared_type == Some(GqlType::Integer))
    );
    assert_eq!(
        analyzed.parameters[2].name.as_str(),
        "other",
        "bare uses remain in the source contract"
    );
    assert_eq!(analyzed.parameters[2].declared_type, None);
}

#[test]
fn request_preflight_handles_missing_inherited_shadowed_and_unused_parameters() {
    let graph = SharedGraph::new(GraphId::new(8101));
    let timestamp = jiff::Timestamp::new(1_788_692_096, 123).unwrap();
    let zone = jiff::tz::TimeZone::UTC;

    let missing = execute(
        &graph,
        "RETURN $missing",
        input([], timestamp, zone.clone()),
    )
    .unwrap_err();
    assert_eq!(missing.gqlstatus().as_str(), "22G03");

    let invalid_unused = execute(
        &graph,
        "RETURN 1",
        input(
            [(
                "unused",
                RequestParameter::new(GqlType::Integer, Value::String(parameter_name("wrong"))),
            )],
            timestamp,
            zone.clone(),
        ),
    )
    .unwrap_err();
    assert_eq!(invalid_unused.gqlstatus().as_str(), "22G03");

    let inherited = execute(
        &graph,
        "RETURN $value AS x_before, $value::INTEGER AS x_declared",
        input(
            [(
                "value",
                RequestParameter::new(GqlType::String, Value::String(parameter_name("text"))),
            )],
            timestamp,
            zone.clone(),
        ),
    )
    .unwrap_err();
    assert_eq!(inherited.gqlstatus().as_str(), "22G03");

    let output = execute(
        &graph,
        "RETURN $value",
        input(
            [
                (
                    "unused",
                    RequestParameter::new(GqlType::Integer, Value::Int(1)),
                ),
                (
                    "value",
                    RequestParameter::new(GqlType::Integer, Value::Int(2)),
                ),
            ],
            timestamp,
            zone,
        ),
    )
    .unwrap();
    assert_eq!(values(output), vec![Value::Int(2)]);
}

#[test]
fn inline_type_failure_precedes_insert_transaction_and_publication() {
    let graph = SharedGraph::new(GraphId::new(8102));
    let before = graph.read();
    let generation = before.meta.generation;
    assert_eq!(before.node_count(), 0);
    drop(before);

    let error = execute(
        &graph,
        "INSERT (:Bad { value: $value::INTEGER }) FINISH",
        input(
            [(
                "value",
                RequestParameter::new(GqlType::String, Value::String(parameter_name("bad"))),
            )],
            jiff::Timestamp::new(1_788_692_096, 0).unwrap(),
            jiff::tz::TimeZone::UTC,
        ),
    )
    .unwrap_err();
    assert_eq!(error.gqlstatus().as_str(), "22G03");
    let after = graph.read();
    assert_eq!(after.meta.generation, generation);
    assert_eq!(after.node_count(), 0);
    drop(after);

    let reference_error = execute(
        &graph,
        "INSERT (:Bad) FINISH",
        input(
            [(
                "stale",
                RequestParameter::new(
                    GqlType::NodeRef,
                    Value::NodeRef(selene_core::NodeId::new(99)),
                ),
            )],
            jiff::Timestamp::new(1_788_692_096, 0).unwrap(),
            jiff::tz::TimeZone::UTC,
        ),
    )
    .unwrap_err();
    assert_eq!(reference_error.gqlstatus().as_str(), "42002");
    let after_reference = graph.read();
    assert_eq!(after_reference.meta.generation, generation);
    assert_eq!(after_reference.node_count(), 0);
}

#[test]
fn one_supplied_instant_reaches_read_and_write_current_datetime_contexts() {
    let graph = SharedGraph::new(GraphId::new(8103));
    let timestamp = jiff::Timestamp::new(1_788_692_096, 123_456_789).unwrap();
    let offset = jiff::tz::Offset::from_seconds(3_600).unwrap();
    let zone = jiff::tz::TimeZone::fixed(offset);
    let zoned = timestamp.to_zoned(zone.clone());

    let read = execute(
        &graph,
        "RETURN CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_DATE, CURRENT_TIME, LOCAL_DATETIME(), LOCAL_TIME()",
        input([], timestamp, zone.clone()),
    )
    .unwrap();
    assert_eq!(
        values(read),
        vec![
            Value::ZonedDateTime(Box::new(zoned.clone())),
            Value::ZonedDateTime(Box::new(zoned.clone())),
            Value::Date(zoned.date()),
            Value::ZonedTime(Box::new(zoned.clone())),
            Value::LocalDateTime(zoned.datetime()),
            Value::LocalTime(zoned.time()),
        ]
    );

    let write = execute(
        &graph,
        "INSERT (n:Clock { at: CURRENT_TIMESTAMP }) RETURN n.at, CURRENT_TIMESTAMP",
        input([], timestamp, zone),
    )
    .unwrap();
    let CatalogSessionOutput::Statement(StatementOutput::Written(write)) = write else {
        panic!("expected write output");
    };
    let table = write.rows.expect("write returns values");
    assert_eq!(
        table.rows()[0].values(),
        &[
            Value::ZonedDateTime(Box::new(zoned.clone())),
            Value::ZonedDateTime(Box::new(zoned)),
        ]
    );
}

#[test]
fn explicit_requests_bypass_plan_caches_that_omit_the_parameter_contract() {
    let graph = SharedGraph::new(GraphId::new(8104));
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(8).unwrap()));
    let registry = BuiltinProcedureRegistry::new();
    let timestamp = jiff::Timestamp::new(1_788_692_096, 0).unwrap();
    let source = "CALL selene.health() YIELD graph_id RETURN $value::INTEGER";

    let first = input(
        [(
            "value",
            RequestParameter::new(GqlType::Integer, Value::Int(1)),
        )],
        timestamp,
        jiff::tz::TimeZone::UTC,
    );
    Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source_catalog_request(source, &registry, first)
        .unwrap();

    let second = input(
        [(
            "value",
            RequestParameter::new(GqlType::String, Value::String(parameter_name("wrong"))),
        )],
        timestamp,
        jiff::tz::TimeZone::UTC,
    );
    let error = Session::new(&graph)
        .with_call_plan_cache(Arc::clone(&cache))
        .execute_source_catalog_request(source, &registry, second)
        .unwrap_err();
    assert_eq!(error.gqlstatus().as_str(), "22G03");
    assert_eq!(cache.stats(), Default::default());
}
