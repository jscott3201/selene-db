//! BRIEF-126 GQLSTATUS Table 8 subclass and warning-channel regressions.

#![cfg(feature = "test-harness")]

mod exec_common;

use std::sync::{Arc, Mutex};

use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value};
use selene_gql::{
    AnalyzedType, BinaryOp, Binding, BindingTableColumn, BindingTableSchema, DataExceptionSubclass,
    EmptyProcedureRegistry, ExecutorWarning, GqlStatus, Session, SourceSpan, ValueExpr,
    WarningSink, analyze, parse,
};
use selene_graph::{GraphTypeDef, NodeTypeDef, SharedGraph, ValidationMode};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn status_for(source: &str) -> String {
    let graph = SharedGraph::new(GraphId::new(12_600));
    let mut session = Session::new(&graph);
    session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect_err("statement errors")
        .gqlstatus()
        .to_string()
}

#[test]
fn data_exception_subclasses_map_to_folded_table8_codes() {
    let cases = [
        (DataExceptionSubclass::StringDataRightTruncation, "22001"),
        (DataExceptionSubclass::NumericValueOutOfRange, "22003"),
        (DataExceptionSubclass::InvalidDatetimeFormat, "22007"),
        (DataExceptionSubclass::DivisionByZero, "22012"),
        (
            DataExceptionSubclass::InvalidArgumentForNaturalLogarithm,
            "2201E",
        ),
        (
            DataExceptionSubclass::InvalidArgumentForPowerFunction,
            "2201F",
        ),
        (DataExceptionSubclass::InvalidValueType, "22G03"),
        (DataExceptionSubclass::ValuesNotComparable, "22G04"),
        (
            DataExceptionSubclass::InvalidDatetimeFunctionFieldName,
            "22G05",
        ),
        (DataExceptionSubclass::InvalidDatetimeFunctionValue, "22G06"),
        (
            DataExceptionSubclass::InvalidDurationFunctionFieldName,
            "22G07",
        ),
        (DataExceptionSubclass::ListDataRightTruncation, "22G0B"),
        (DataExceptionSubclass::ListElementError, "22G0C"),
        (DataExceptionSubclass::InvalidDurationFormat, "22G0H"),
        (DataExceptionSubclass::PathDataRightTruncation, "22G10"),
        (
            DataExceptionSubclass::IncompatibleTemporalInstantUnitGroups,
            "22G14",
        ),
        (
            DataExceptionSubclass::MultipleAssignmentsToGraphElementProperty,
            "22G0M",
        ),
        (
            DataExceptionSubclass::NodeLabelsBelowSupportedMinimum,
            "22G0N",
        ),
        (
            DataExceptionSubclass::NodeLabelsExceedSupportedMaximum,
            "22G0P",
        ),
        (
            DataExceptionSubclass::EdgeLabelsBelowSupportedMinimum,
            "22G0Q",
        ),
        (
            DataExceptionSubclass::EdgeLabelsExceedSupportedMaximum,
            "22G0R",
        ),
        (
            DataExceptionSubclass::NodePropertiesExceedSupportedMaximum,
            "22G0S",
        ),
        (
            DataExceptionSubclass::EdgePropertiesExceedSupportedMaximum,
            "22G0T",
        ),
        (DataExceptionSubclass::RecordDataFieldUnassignable, "22G0X"),
        (DataExceptionSubclass::MalformedPath, "22G0Z"),
    ];

    for (subclass, expected) in cases {
        assert_eq!(subclass.gqlstatus().as_str(), expected);
    }
}

#[test]
fn runtime_data_exceptions_emit_specific_subclasses() {
    let cases = [
        ("RETURN 9223372036854775807 + 1 AS v", "22003"),
        ("RETURN 1 / 0 AS v", "22012"),
        ("RETURN power(0, -1) AS v", "2201F"),
        ("RETURN sqrt(-1) AS v", "2201F"),
        ("RETURN 'x' + 1 AS v", "22G03"),
        ("RETURN DURATION('P1M') + DURATION('PT1H') AS v", "22G14"),
        ("RETURN {a: 1, a: 2} AS v", "22G0X"),
    ];

    for (source, expected) in cases {
        assert_eq!(status_for(source), expected, "source: {source}");
    }
}

#[test]
fn dynamic_ordering_of_incomparable_values_emits_22g04() {
    let lhs = db_string("lhs");
    let rhs = db_string("rhs");
    let expr = ValueExpr::BinaryOp {
        op: BinaryOp::Lt,
        lhs: Box::new(ValueExpr::Variable {
            name: lhs.clone(),
            span: SourceSpan::new(0, 3),
        }),
        rhs: Box::new(ValueExpr::Variable {
            name: rhs.clone(),
            span: SourceSpan::new(6, 3),
        }),
        span: SourceSpan::new(0, 9),
    };
    let schema = BindingTableSchema {
        columns: vec![
            BindingTableColumn {
                name: Some(lhs),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
            BindingTableColumn {
                name: Some(rhs),
                hidden: None,
                ty: AnalyzedType::DYNAMIC,
            },
        ],
    };
    let binding = Binding::new([Value::Int(1), Value::String(db_string("x"))]);
    let caps = selene_gql::ImplDefinedCaps::default();
    let ctx = exec_common::empty_graph_context(&caps);

    let error = selene_gql::runtime::evaluate_for_test(&expr, &binding, &schema, &ctx)
        .expect_err("dynamic ordering errors");

    assert_eq!(error.gqlstatus().as_str(), "22G04");
}

#[test]
fn transaction_class_codes_use_live_table8_subclasses() {
    let graph = SharedGraph::new(GraphId::new(12_601));
    let mut session = Session::new(&graph);

    session
        .execute_source("START TRANSACTION", &EmptyProcedureRegistry)
        .expect("first start succeeds");
    let active = session
        .execute_source("START TRANSACTION", &EmptyProcedureRegistry)
        .expect_err("nested start errors");
    assert_eq!(active.gqlstatus(), GqlStatus::ACTIVE_TRANSACTION);
    session.abort();

    let no_active = session
        .execute_source("COMMIT", &EmptyProcedureRegistry)
        .expect_err("commit without active transaction errors");
    assert_eq!(
        no_active.gqlstatus(),
        GqlStatus::INVALID_TRANSACTION_TERMINATION
    );
}

#[test]
fn detach_delete_requirement_emits_g1001() {
    let graph = SharedGraph::new(GraphId::new(12_602));
    {
        let mut txn = graph.begin_write();
        let mut mutator = txn.mutator();
        let victim = mutator
            .create_node(LabelSet::single(db_string("Victim")), PropertyMap::new())
            .expect("victim inserts");
        let other = mutator
            .create_node(LabelSet::single(db_string("Other")), PropertyMap::new())
            .expect("other inserts");
        mutator
            .create_edge(db_string("REL"), victim, other, PropertyMap::new())
            .expect("edge inserts");
        txn.commit().expect("fixture commits");
    }
    let mut session = Session::new(&graph);

    let error = session
        .execute_source("MATCH (n:Victim) DELETE n FINISH", &EmptyProcedureRegistry)
        .expect_err("bare delete with incident edge errors");

    assert_eq!(error.gqlstatus(), GqlStatus::DEPENDENT_OBJECT_STILL_EXISTS);
}

#[test]
fn closed_graph_schema_analysis_emits_g2000() {
    let person = db_string("Person");
    let graph_type = GraphTypeDef {
        name: db_string("schema.graph"),
        node_types: vec![NodeTypeDef {
            name: person.clone(),
            key_labels: LabelSet::single(person),
            properties: Vec::new(),
            validation_mode: ValidationMode::Strict,
        }],
        edge_types: Vec::new(),
    };
    let statement = parse("INSERT (n:Project)").expect("test input parses");

    let error = analyze(statement, &EmptyProcedureRegistry, Some(&graph_type))
        .expect_err("unknown node type rejects");

    assert_eq!(error.gqlstatus(), GqlStatus::GRAPH_TYPE_VIOLATION);
}

#[derive(Clone)]
struct RecordingSink {
    warnings: Arc<Mutex<Vec<ExecutorWarning>>>,
}

impl WarningSink for RecordingSink {
    fn emit(&mut self, warning: ExecutorWarning) {
        self.warnings.lock().expect("warning mutex").push(warning);
    }
}

#[test]
fn aggregate_null_skip_emits_01g11_once_per_aggregate_expression() {
    let graph = SharedGraph::new(GraphId::new(12_603));
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        warnings: Arc::clone(&warnings),
    };
    let mut session = Session::new(&graph).with_warning_sink(sink);

    session
        .execute_source(
            "UNWIND [1, NULL, NULL, 2] AS x RETURN sum(x) AS s",
            &EmptyProcedureRegistry,
        )
        .expect("sum executes");
    let observed = warnings.lock().expect("warning mutex").clone();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0].code,
        GqlStatus::NULL_VALUE_ELIMINATED_IN_SET_FUNCTION
    );

    warnings.lock().expect("warning mutex").clear();
    session
        .execute_source(
            "UNWIND [1, NULL] AS x RETURN count(x) AS c",
            &EmptyProcedureRegistry,
        )
        .expect("count executes");
    let observed = warnings.lock().expect("warning mutex").clone();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0].code,
        GqlStatus::NULL_VALUE_ELIMINATED_IN_SET_FUNCTION
    );
}

#[test]
fn default_warning_sink_keeps_null_skip_non_fatal() {
    let graph = SharedGraph::new(GraphId::new(12_604));
    let mut session = Session::new(&graph);

    session
        .execute_source(
            "UNWIND [NULL] AS x RETURN count(x) AS c",
            &EmptyProcedureRegistry,
        )
        .expect("default sink discards warning");
}
