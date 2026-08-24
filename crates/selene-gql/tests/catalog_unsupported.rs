//! Database-catalog statements reaching a bare lower-engine session.
//!
//! The lower engine owns one graph and no catalog. `CREATE/DROP SCHEMA` and
//! `CREATE GRAPH` must report a structured implementation-defined error and
//! change nothing; they must not silently no-op. `DROP GRAPH` keeps its
//! pre-existing `IM_DROP_GRAPH` factory reset of the session graph.

use selene_core::GraphId;
use selene_gql::{EmptyProcedureRegistry, ExecutorError, GqlStatus, Session, StatementOutput};
use selene_graph::SharedGraph;

#[test]
fn bare_lower_session_rejects_database_catalog_statements_without_state_change() {
    let graph = SharedGraph::new(GraphId::new(4400));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("seed insert succeeds");
    let schema_version = graph.schema_version();
    for source in [
        "CREATE SCHEMA /memory",
        "CREATE SCHEMA IF NOT EXISTS /memory",
        "DROP SCHEMA /memory",
        "DROP SCHEMA IF EXISTS /memory",
        "CREATE GRAPH g ANY",
        "CREATE GRAPH IF NOT EXISTS /memory/g ANY",
    ] {
        let error = session
            .execute_source(source, &EmptyProcedureRegistry)
            .expect_err(source);
        assert!(
            matches!(
                &error,
                ExecutorError::ImplementationDefined { detail }
                    if detail.contains("database catalog statements require the database facade")
            ),
            "{source}: expected structured implementation-defined error, got {error:?}"
        );
        assert_eq!(
            error.gqlstatus(),
            GqlStatus::IMPLEMENTATION_DEFINED_ERROR,
            "{source}"
        );
        assert_eq!(graph.schema_version(), schema_version, "{source}");
        assert_eq!(graph.read().node_count(), 1, "{source}");
    }
}

#[test]
fn bare_lower_session_drop_graph_keeps_the_factory_reset() {
    let graph = SharedGraph::new(GraphId::new(4401));
    let mut session = Session::new(&graph);
    session
        .execute_source("INSERT (:Person)", &EmptyProcedureRegistry)
        .expect("seed insert succeeds");
    let output = session
        .execute_source("DROP GRAPH IF EXISTS anything", &EmptyProcedureRegistry)
        .expect("lower-engine DROP GRAPH factory-resets");
    assert!(matches!(output, StatementOutput::Written(_)));
    assert_eq!(graph.read().node_count(), 0);
}
