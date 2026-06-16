//! ISO/IEC 39075:2024 section 7 session-management behavior tests.
//!
//! Exercises the implemented session subset end to end: SET VALUE (GS03),
//! SET TIME ZONE (GS15) threaded into the section 20.27 current-datetime
//! functions, SET GRAPH to current-graph expressions (section 7.1), RESET
//! targets (GS04/GS07/GS08/GS16), SESSION CLOSE (section 7.3) with its
//! termination guard, IF NOT EXISTS (section 7.4), the flagger feature stamps,
//! and the D1-deferred schema / graph-parameter forms failing cleanly.

use selene_core::GraphId;
use selene_core::feature_register::{
    ANNEX_B_REGISTER, FeatureId, NOT_SUPPORTED_RATIONALE, SUPPORTED_FEATURES,
};
use selene_gql::{
    EmptyProcedureRegistry, ExecutorError, GqlStatus, GqlType, ParserError, Session,
    SessionSetGraphTarget, Statement, StatementOutput, Value, analyze, execute_statement,
    feature_walk, parse, plan,
};
use selene_graph::SharedGraph;

fn graph(id: u64) -> SharedGraph {
    SharedGraph::new(GraphId::new(id))
}

fn run(session: &mut Session<'_>, source: &str) -> Result<StatementOutput, ExecutorError> {
    session.execute_source(source, &EmptyProcedureRegistry)
}

fn single_value(output: StatementOutput) -> Value {
    let StatementOutput::Rows(table) = output else {
        panic!("expected row output, got {output:?}");
    };
    let row = table.rows().first().expect("at least one row");
    row.values().first().expect("at least one column").clone()
}

#[path = "session_management/close.rs"]
mod close;
#[path = "session_management/deferred.rs"]
mod deferred;
#[path = "session_management/flags.rs"]
mod flags;
#[path = "session_management/graph_target.rs"]
mod graph_target;
#[path = "session_management/registry.rs"]
mod registry;
#[path = "session_management/reset.rs"]
mod reset;
#[path = "session_management/set_value.rs"]
mod set_value;
#[path = "session_management/time_zone.rs"]
mod time_zone;

fn unbound_parameter(session: &mut Session<'_>) -> bool {
    matches!(
        run(session, "RETURN $p"),
        Err(ExecutorError::UnboundParameter { .. })
    )
}

fn walked_features(source: &str) -> Vec<FeatureId> {
    let statement = parse(source).expect("parse");
    feature_walk(&statement)
        .into_iter()
        .map(|use_| use_.feature_id)
        .collect()
}
