//! Facade request lifecycle and uniform outcome coverage.

use std::error::Error as _;

use selene_db::{
    CreatePolicy, Database, ErrorKind, ObjectPath, Request, RequestOutcome, RequestSlotState,
    SchemaPath,
};

fn fixture(name: &str) -> (Database, ObjectPath, selene_db::Session) {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema = SchemaPath::regular("selene", name).unwrap();
    let graph = ObjectPath::regular("selene", name, "main").unwrap();
    catalog
        .create_schema(&schema, CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&graph, None, CreatePolicy::Strict)
        .unwrap();
    let session = database.session(&graph).unwrap();
    (database, graph, session)
}

fn failed(outcome: &RequestOutcome) -> &selene_db::Error {
    outcome.error().expect("request must fail")
}

#[test]
fn session_remains_send() {
    fn require_send<T: Send>() {}
    require_send::<selene_db::Session>();
}

#[test]
fn compilation_runtime_validation_and_catalog_failures_use_request_outcome() {
    let (_database, _graph, session) = fixture("request_failures");
    let cases = [
        ("RETURN (", "42001"),
        ("RETURN missing", "42N03"),
        ("MATCH (a)-[:K*1..101]->(b) RETURN a", "5GQL1"),
        ("RETURN 1 / 0", "22012"),
        ("RETURN $missing", "22G03"),
    ];
    for (source, status) in cases {
        let outcome = session.execute_request(Request::new(source));
        let error = failed(&outcome);
        assert_eq!(error.gqlstatus().unwrap().as_str(), status, "{source}");
        assert!(
            error.source().is_some(),
            "engine source is retained for {source}"
        );
        assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
        assert!(session.context().current_request().is_none());
    }

    let catalog = session.execute_request(Request::new("CREATE SCHEMA /request_failures"));
    let error = failed(&catalog);
    assert_eq!(error.kind(), ErrorKind::CatalogObjectAlreadyExists);
    assert_eq!(error.gqlstatus().unwrap().as_str(), "42N10");
    assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
}

#[test]
fn stale_selection_is_a_failed_request_and_retains_its_context() {
    let (database, graph, session) = fixture("stale_request");
    database
        .catalog()
        .drop_graph(&graph, selene_db::DropPolicy::Strict)
        .unwrap();

    let outcome = session.execute_request(Request::new("RETURN 1"));
    assert_eq!(failed(&outcome).kind(), ErrorKind::StaleSessionReference);
    assert!(outcome.context().parameters().is_empty());
    assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
}

#[test]
fn returned_failures_clear_the_slot_under_hostile_repetition() {
    let (_database, _graph, session) = fixture("request_repetition");
    for _ in 0..256 {
        assert!(matches!(
            session.execute_request(Request::new("RETURN (")),
            RequestOutcome::Failed { .. }
        ));
        assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);
    }
    assert!(matches!(
        session.execute_request(Request::new("RETURN 1")),
        RequestOutcome::Succeeded { .. }
    ));
}

#[test]
fn legacy_execute_is_a_source_compatible_request_adapter() {
    let (_database, _graph, session) = fixture("legacy_adapter");
    let legacy = session.execute("RETURN 1").unwrap();
    let request = session.execute_request(Request::new("RETURN 1"));

    assert_eq!(request.execution(), Some(&legacy));
    assert_eq!(session.context().request_slot(), RequestSlotState::Vacant);

    let legacy_error = session.execute("RETURN 1 / 0").unwrap_err();
    let request_error = session
        .execute_request(Request::new("RETURN 1 / 0"))
        .error()
        .expect("request fails")
        .to_string();
    assert_eq!(request_error, legacy_error.message());
    assert_eq!(legacy_error.gqlstatus().unwrap().as_str(), "22012");
}
