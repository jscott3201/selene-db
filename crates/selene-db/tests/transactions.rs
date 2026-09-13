//! Facade transaction demarcation, visibility, access, mixing, and conflicts.

use selene_db::{
    CreatePolicy, Database, DropPolicy, Error, ErrorKind, ObjectPath, SchemaPath, Session,
    TransactionAccessMode, TransactionSlotState, TransactionState, WriteSummary,
};

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn graph(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

fn fixture(name: &str) -> (Database, ObjectPath, Session, Session) {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let path = graph(name, "main");
    catalog
        .create_schema(&schema(name), CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap();
    let first = database.session(&path).unwrap();
    let second = database.session(&path).unwrap();
    (database, path, first, second)
}

fn status(error: &Error) -> String {
    error
        .gqlstatus()
        .expect("transaction error has status")
        .as_str()
        .to_owned()
}

fn count(session: &Session, label: &str) -> usize {
    session
        .execute(&format!("MATCH (n:{label}) RETURN n"))
        .unwrap()
        .row_count()
        .unwrap()
}

#[test]
fn explicit_multi_request_visibility_and_gql_demarcation_publish_once() {
    let (_database, _path, session, observer) = fixture("tx_visibility");

    session.execute("START TRANSACTION").unwrap();
    let started = session.context().transaction().unwrap();
    assert_eq!(started.state(), TransactionState::Active);
    assert_eq!(started.access_mode(), TransactionAccessMode::ReadWrite);
    assert_ne!(started.id().get(), 0);

    session
        .execute("INSERT (:TxVisible { name: 'a' })")
        .unwrap();
    assert_eq!(count(&session, "TxVisible"), 1);
    session
        .execute("INSERT (:TxVisible { name: 'b' })")
        .unwrap();
    assert_eq!(count(&session, "TxVisible"), 2);
    assert_eq!(count(&observer, "TxVisible"), 0);

    session.execute("COMMIT").unwrap();
    let committed = session.context().transaction().unwrap();
    assert_eq!(committed.state(), TransactionState::Committed);
    assert_eq!(committed.statement_count(), 4);
    assert_eq!(committed.staged_change_count(), 2);
    assert_eq!(count(&observer, "TxVisible"), 2);

    let error = session.execute("COMMIT").unwrap_err();
    assert_eq!(status(&error), "2D000");
    let error = session.execute("ROLLBACK").unwrap_err();
    assert_eq!(status(&error), "2D000");
}

#[test]
fn rust_demarcation_guards_and_terminal_restart_use_one_state_machine() {
    let (_database, _path, session, _observer) = fixture("tx_rust");

    let first = session
        .start_transaction(TransactionAccessMode::ReadWrite)
        .unwrap();
    let error = session
        .start_transaction(TransactionAccessMode::ReadOnly)
        .unwrap_err();
    assert_eq!(status(&error), "25G01");
    assert_eq!(session.context().transaction().unwrap().id(), first.id());
    let rolled_back = session.rollback_transaction().unwrap();
    assert_eq!(rolled_back.state(), TransactionState::RolledBack);
    assert_eq!(
        session.context().transaction_slot(),
        TransactionSlotState::RolledBack
    );
    let error = session.rollback_transaction().unwrap_err();
    assert_eq!(status(&error), "2D000");

    let second = session
        .start_transaction(TransactionAccessMode::ReadOnly)
        .unwrap();
    assert!(second.id().get() > first.id().get());
    assert_eq!(
        session.commit_transaction().unwrap().state(),
        TransactionState::Committed
    );
}

#[test]
fn read_only_snapshot_serializes_before_writer_and_commit_does_not_store() {
    let (database, _path, reader, writer) = fixture("tx_read_only");
    let before_start = database.catalog().snapshot();
    reader
        .start_transaction(TransactionAccessMode::ReadOnly)
        .unwrap();
    assert!(
        database
            .catalog()
            .snapshot()
            .shares_state_with(&before_start)
    );

    writer.execute("INSERT (:ReadPinned)").unwrap();
    assert_eq!(count(&writer, "ReadPinned"), 1);
    assert_eq!(count(&reader, "ReadPinned"), 0);
    let after_writer = database.catalog().snapshot();

    let committed = reader.commit_transaction().unwrap();
    assert_eq!(committed.state(), TransactionState::Committed);
    assert!(
        database
            .catalog()
            .snapshot()
            .shares_state_with(&after_writer)
    );
}

#[test]
fn read_only_write_fails_and_failed_commit_rolls_back_with_25n02() {
    let (database, _path, reader, observer) = fixture("tx_read_only_write");
    let before = database.catalog().snapshot();
    reader
        .start_transaction(TransactionAccessMode::ReadOnly)
        .unwrap();
    let error = reader.execute("INSERT (:Forbidden)").unwrap_err();
    assert_eq!(status(&error), "25G03");
    assert_eq!(
        reader.context().transaction_slot(),
        TransactionSlotState::Failed
    );
    assert!(database.catalog().snapshot().shares_state_with(&before));
    assert_eq!(count(&observer, "Forbidden"), 0);

    let error = reader.commit_transaction().unwrap_err();
    assert_eq!(status(&error), "25N02");
    assert_eq!(
        reader.context().transaction_slot(),
        TransactionSlotState::RolledBack
    );
}

#[test]
fn implicit_and_explicit_data_and_database_catalog_writes_are_equivalent() {
    let (database, _path, implicit, explicit) = fixture("tx_equivalence");
    let implicit_write = implicit.execute("INSERT (:Equivalent)").unwrap();
    assert_eq!(
        implicit_write.write_summary(),
        Some(WriteSummary::new(1, None))
    );

    explicit.execute("START TRANSACTION").unwrap();
    let explicit_write = explicit.execute("INSERT (:Equivalent)").unwrap();
    assert_eq!(
        explicit_write.write_summary(),
        implicit_write.write_summary()
    );
    explicit.execute("COMMIT").unwrap();
    assert_eq!(count(&implicit, "Equivalent"), 2);

    implicit.execute("CREATE SCHEMA /implicit_schema").unwrap();
    explicit.execute("START TRANSACTION").unwrap();
    explicit.execute("CREATE SCHEMA /explicit_schema").unwrap();
    assert!(
        database
            .catalog()
            .snapshot()
            .resolve_schema(&schema("explicit_schema"))
            .is_err()
    );
    explicit.execute("COMMIT").unwrap();
    let snapshot = database.catalog().snapshot();
    snapshot.resolve_schema(&schema("implicit_schema")).unwrap();
    snapshot.resolve_schema(&schema("explicit_schema")).unwrap();
}

#[test]
fn gp18_rejects_both_mixing_orders_and_same_category_stages_repeatedly() {
    let (database, _path, session, observer) = fixture("tx_mixing");

    session.execute("START TRANSACTION").unwrap();
    session.execute("INSERT (:MixedData)").unwrap();
    let error = session.execute("CREATE SCHEMA /mixed_catalog").unwrap_err();
    assert_eq!(status(&error), "25G02");
    assert_eq!(count(&observer, "MixedData"), 0);
    assert!(
        database
            .catalog()
            .snapshot()
            .resolve_schema(&schema("mixed_catalog"))
            .is_err()
    );
    session.execute("ROLLBACK").unwrap();

    session.execute("START TRANSACTION").unwrap();
    session.execute("CREATE SCHEMA /mixed_first").unwrap();
    let error = session.execute("INSERT (:MixedAfterCatalog)").unwrap_err();
    assert_eq!(status(&error), "25G02");
    session.execute("ROLLBACK").unwrap();
    assert!(
        database
            .catalog()
            .snapshot()
            .resolve_schema(&schema("mixed_first"))
            .is_err()
    );

    session.execute("START TRANSACTION").unwrap();
    session.execute("CREATE SCHEMA /same_one").unwrap();
    session.execute("CREATE SCHEMA /same_two").unwrap();
    session.execute("COMMIT").unwrap();
    let snapshot = database.catalog().snapshot();
    snapshot.resolve_schema(&schema("same_one")).unwrap();
    snapshot.resolve_schema(&schema("same_two")).unwrap();
}

#[test]
fn parse_analysis_runtime_procedure_and_staging_failures_preserve_original() {
    let (database, _path, session, _observer) = fixture("tx_failure");
    session.execute("START TRANSACTION").unwrap();
    session.execute("INSERT (:ParseStaged)").unwrap();
    let parse = session.execute("RETURN (").unwrap_err();
    assert_ne!(status(&parse), "25N02");
    assert_eq!(
        session.context().transaction_slot(),
        TransactionSlotState::Failed
    );
    let failed = session.execute("RETURN 1").unwrap_err();
    assert_eq!(status(&failed), "25N02");
    let start = session.execute("START TRANSACTION").unwrap_err();
    assert_eq!(status(&start), "25G01");
    session.execute("ROLLBACK").unwrap();
    assert_eq!(count(&session, "ParseStaged"), 0);

    for (source, expected) in [
        ("RETURN missing", None),
        ("RETURN 1 / 0", Some("22012")),
        ("CALL missing.procedure()", Some("42N04")),
    ] {
        session.execute("START TRANSACTION").unwrap();
        session.execute("INSERT (:FailureStaged)").unwrap();
        let original = session.execute(source).unwrap_err();
        assert_ne!(status(&original), "25N02");
        if let Some(expected) = expected {
            assert_eq!(status(&original), expected);
        }
        assert_eq!(
            session.context().transaction_slot(),
            TransactionSlotState::Failed
        );
        let failed = session.execute("RETURN 1").unwrap_err();
        assert_eq!(status(&failed), "25N02");
        session.execute("ROLLBACK").unwrap();
        assert_eq!(count(&session, "FailureStaged"), 0);
    }

    session.execute("START TRANSACTION").unwrap();
    session.execute("CREATE SCHEMA /discarded_catalog").unwrap();
    let duplicate = session.execute("CREATE SCHEMA /tx_failure").unwrap_err();
    assert_eq!(status(&duplicate), "42N10");
    assert_eq!(
        session.context().transaction_slot(),
        TransactionSlotState::Failed
    );
    session.execute("ROLLBACK").unwrap();
    assert!(
        database
            .catalog()
            .snapshot()
            .resolve_schema(&schema("tx_failure"))
            .is_ok()
    );
    assert!(
        database
            .catalog()
            .snapshot()
            .resolve_schema(&schema("discarded_catalog"))
            .is_err()
    );
}

#[test]
fn selected_maintenance_fails_active_transaction_without_publication() {
    let (database, _path, session, _observer) = fixture("tx_maintenance");
    let before = database.catalog().snapshot();
    session.execute("START TRANSACTION").unwrap();
    let error = session.execute("CALL selene.compact()").unwrap_err();
    assert_eq!(status(&error), "42N01");
    assert_eq!(
        session.context().transaction_slot(),
        TransactionSlotState::Failed
    );
    assert!(database.catalog().snapshot().shares_state_with(&before));
    session.execute("ROLLBACK").unwrap();
}

#[test]
fn selected_writer_conflict_returns_40000_without_lost_update() {
    let (_database, _path, loser, winner) = fixture("tx_conflict_write");
    loser.execute("START TRANSACTION").unwrap();
    loser.execute("INSERT (:Loser)").unwrap();
    winner.execute("INSERT (:Winner)").unwrap();

    let error = loser.execute("COMMIT").unwrap_err();
    assert_eq!(status(&error), "40000");
    assert_eq!(
        loser.context().transaction_slot(),
        TransactionSlotState::RolledBack
    );
    assert_eq!(count(&winner, "Winner"), 1);
    assert_eq!(count(&winner, "Loser"), 0);
}

#[test]
fn direct_catalog_conflict_wins_and_transaction_catalog_delta_is_discarded() {
    let (database, _path, session, _observer) = fixture("tx_conflict_catalog");
    session.execute("START TRANSACTION").unwrap();
    session.execute("CREATE SCHEMA /staged_loser").unwrap();
    database
        .catalog()
        .create_schema(&schema("direct_winner"), CreatePolicy::Strict)
        .unwrap();

    let error = session.execute("COMMIT").unwrap_err();
    assert_eq!(status(&error), "40000");
    let snapshot = database.catalog().snapshot();
    snapshot.resolve_schema(&schema("direct_winner")).unwrap();
    assert!(snapshot.resolve_schema(&schema("staged_loser")).is_err());
}

#[test]
fn catalog_graph_lifecycle_deltas_stage_atomically_with_type_binding_and_replace() {
    let (database, _path, session, _observer) = fixture("tx_lifecycle");
    let catalog = database.catalog();

    session.execute("START TRANSACTION").unwrap();
    session
        .execute("CREATE GRAPH TYPE local_shape { NODE TYPE Person () }")
        .unwrap();
    session
        .execute("CREATE GRAPH local_graph TYPED local_shape")
        .unwrap();
    let before = catalog.snapshot();
    assert!(
        before
            .resolve_graph(&graph("tx_lifecycle", "local_graph"))
            .is_err()
    );
    session.execute("COMMIT").unwrap();

    let published = catalog.snapshot();
    let original = published
        .resolve_graph(&graph("tx_lifecycle", "local_graph"))
        .unwrap();
    let graph_type = published
        .resolve_graph_type(&graph("tx_lifecycle", "local_shape"))
        .unwrap();
    assert_eq!(
        original.graph_type.map(|id| id.get()),
        Some(graph_type.id.get())
    );

    session.execute("START TRANSACTION").unwrap();
    session
        .execute("CREATE OR REPLACE GRAPH local_graph TYPED local_shape")
        .unwrap();
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&graph("tx_lifecycle", "local_graph"))
            .unwrap()
            .id,
        original.id
    );
    session.execute("COMMIT").unwrap();
    let replaced = catalog
        .snapshot()
        .resolve_graph(&graph("tx_lifecycle", "local_graph"))
        .unwrap();
    assert!(replaced.id.get() > original.id.get());

    session.execute("START TRANSACTION").unwrap();
    session.execute("DROP GRAPH local_graph").unwrap();
    session.execute("ROLLBACK").unwrap();
    assert_eq!(
        catalog
            .snapshot()
            .resolve_graph(&graph("tx_lifecycle", "local_graph"))
            .unwrap()
            .id,
        replaced.id
    );
}

#[test]
fn dropping_a_session_with_staged_work_publishes_nothing() {
    let (_database, _path, session, observer) = fixture("tx_drop");
    session.execute("START TRANSACTION").unwrap();
    session.execute("INSERT (:DroppedSession)").unwrap();
    drop(session);
    assert_eq!(count(&observer, "DroppedSession"), 0);
}

#[test]
fn stale_failed_and_terminal_controls_need_no_retained_graph_snapshot() {
    let (database, path, session, _observer) = fixture("tx_stale_control");
    let original = database.catalog().snapshot().resolve_graph(&path).unwrap();

    session.execute("START TRANSACTION").unwrap();
    session.execute("RETURN (").unwrap_err();
    assert_eq!(
        session.context().transaction_slot(),
        TransactionSlotState::Failed
    );

    database
        .catalog()
        .drop_graph(&path, DropPolicy::Strict)
        .unwrap();
    let replacement = database
        .catalog()
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap();
    let selene_db::CreateOutcome::Created(replacement) = replacement else {
        panic!("recreated graph must have a fresh identity");
    };
    assert_ne!(replacement.id, original.id);

    let rollback = session.execute("ROLLBACK").unwrap();
    assert_eq!(rollback.diagnostics().primary().status().as_str(), "00001");
    let rolled_back = session.context().transaction().unwrap();
    assert_eq!(rolled_back.state(), TransactionState::RolledBack);
    assert_eq!(rolled_back.selected_graph(), original.id);

    for source in ["COMMIT", "ROLLBACK"] {
        let error = session.execute(source).unwrap_err();
        assert_eq!(status(&error), "2D000", "{source}");
    }
    let start = session.execute("START TRANSACTION").unwrap_err();
    assert_eq!(start.kind(), ErrorKind::StaleSessionReference);
    assert_eq!(session.context().transaction().unwrap(), rolled_back);

    let replacement_session = database.session(&path).unwrap();
    let started = replacement_session
        .start_transaction(TransactionAccessMode::ReadWrite)
        .unwrap();
    assert_eq!(started.selected_graph(), replacement.id);
    replacement_session.rollback_transaction().unwrap();
}
