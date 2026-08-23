//! Public facade behavior at the consumer boundary.

use std::error::Error as _;

use selene_db::{
    Database, DatabaseBuilder, DatabaseConfig, ErrorKind, ExecutionOutcome, GqlStatus, OpenMode,
    Session, WriteSummary,
};

fn row_count(outcome: ExecutionOutcome) -> usize {
    let ExecutionOutcome::Rows { row_count } = outcome else {
        panic!("expected row summary, got {outcome:?}");
    };
    row_count
}

#[test]
fn facade_writes_and_queries_the_bootstrap_graph() {
    let database = Database::builder().build();
    let session = database.session();

    assert_eq!(
        session
            .execute("INSERT (:Person { name: 'Ada' })")
            .expect("insert succeeds"),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        row_count(
            session
                .execute("MATCH (n:Person) RETURN n")
                .expect("query succeeds")
        ),
        1
    );
}

#[test]
fn facade_summarizes_write_return_outcomes() {
    let database = Database::builder().build();
    let session = database.session();

    assert_eq!(
        session
            .execute("INSERT (n:Person) RETURN n")
            .expect("write with return succeeds"),
        ExecutionOutcome::Written(WriteSummary::new(1, Some(1)))
    );
}

#[test]
fn session_owns_database_after_originating_handle_is_dropped() {
    fn assert_send_static<T: Send + 'static>(_: &T) {}

    let session = {
        let database = Database::builder().build();
        let session = database.session();
        drop(database);
        session
    };
    assert_send_static(&session);

    assert_eq!(
        row_count(session.execute("RETURN 1").expect("session remains usable")),
        1
    );
}

#[test]
fn sessions_share_data_and_drop_graph_resets_repeatedly() {
    let database = Database::builder().build();
    database
        .session()
        .execute("INSERT (:Person { name: 'Ada' })")
        .expect("insert succeeds");
    assert_eq!(
        row_count(
            database
                .session()
                .execute("MATCH (n:Person) RETURN n")
                .expect("second session sees insert")
        ),
        1
    );

    assert_eq!(
        database
            .session()
            .execute("DROP GRAPH default")
            .expect("drop resets bootstrap graph"),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        row_count(
            database
                .session()
                .execute("MATCH (n:Person) RETURN n")
                .expect("fresh session sees reset")
        ),
        0
    );
    assert_eq!(
        database
            .session()
            .execute("DROP GRAPH default")
            .expect("repeated reset succeeds"),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        row_count(
            database
                .session()
                .execute("MATCH (n) RETURN n")
                .expect("repeated reset leaves graph empty")
        ),
        0
    );
}

#[test]
fn drop_graph_reclaims_algorithm_projection_state() {
    let database = Database::builder().build();
    let session = database.session();

    assert_eq!(
        session
            .execute("CALL algo.projection_build('facade_projection', NULL, NULL, NULL)")
            .expect("projection build succeeds"),
        ExecutionOutcome::Empty
    );
    assert_eq!(
        row_count(
            session
                .execute("CALL algo.projection_list() YIELD name")
                .expect("projection list succeeds before reset")
        ),
        1
    );
    session
        .execute("CALL algo.projection_get('missing') YIELD name")
        .expect_err("missing projection fails without clearing catalog state");
    assert_eq!(
        row_count(
            session
                .execute("CALL algo.projection_list() YIELD name")
                .expect("projection survives an execution error")
        ),
        1
    );

    assert_eq!(
        session
            .execute("DROP GRAPH default")
            .expect("drop resets bootstrap graph"),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        row_count(
            session
                .execute("CALL algo.projection_list() YIELD name")
                .expect("projection list succeeds after reset")
        ),
        0
    );
}

#[test]
fn stateful_controls_are_rejected_without_poisoning_facade_session() {
    let database = Database::builder().build();
    let session = database.session();
    let controls = [
        "START TRANSACTION",
        "COMMIT",
        "ROLLBACK",
        "SESSION SET VALUE $answer = 42",
        "SESSION CLOSE",
    ];

    for source in controls.into_iter().chain(controls) {
        let error = session.execute(source).expect_err(source);
        assert_eq!(error.kind(), ErrorKind::FeatureNotSupported, "{source}");
        assert_eq!(
            error.gqlstatus(),
            Some(GqlStatus::FEATURE_NOT_SUPPORTED),
            "{source}"
        );
    }

    assert_eq!(
        row_count(
            session
                .execute("RETURN 1")
                .expect("normal query works after repeated rejection")
        ),
        1
    );
    assert_eq!(
        session
            .execute("INSERT (:Person)")
            .expect("write remains auto-committed"),
        ExecutionOutcome::Written(WriteSummary::new(1, None))
    );
    assert_eq!(
        row_count(
            database
                .session()
                .execute("MATCH (n:Person) RETURN n")
                .expect("write did not enter hidden transaction state")
        ),
        1
    );
}

#[test]
fn invalid_gql_maps_to_owned_facade_error() {
    let database = Database::builder().build();
    let error = database
        .session()
        .execute("NOT A GQL STATEMENT")
        .expect_err("invalid source fails");

    assert_eq!(error.kind(), ErrorKind::InvalidGql);
    assert_eq!(
        error.gqlstatus().map(|status| status.as_str().to_owned()),
        Some("42001".to_owned())
    );
    assert!(!error.message().is_empty());
    assert!(error.source().is_some());
}

#[test]
fn builder_exposes_only_consumed_in_memory_configuration() {
    let config = DatabaseConfig::default();
    let builder = DatabaseBuilder::from_config(config.clone());

    assert_eq!(config.open_mode(), OpenMode::InMemory);
    assert_eq!(builder.config(), &config);
    assert_eq!(builder.build().config(), &config);
}

#[test]
fn facade_session_type_has_no_lifetime_parameter() {
    fn move_session(session: Session) -> Session {
        session
    }

    let database = Database::builder().build();
    let session = move_session(database.session());
    assert_eq!(
        row_count(session.execute("RETURN 1").expect("moved session works")),
        1
    );
}
