use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use selene_catalog::{
    CatalogDescriptor, CatalogObjectId, CatalogTransaction, CreationMetadata,
    GraphId as LowerGraphId, SchemaId as LowerSchemaId,
};
use selene_core::{GraphId as CoreGraphId, LabelSet, PropertyMap};
use selene_graph::SharedGraph;

use super::{AuthorityOutcome, DatabaseDraft};
use crate::{
    CreatePolicy, Database, ObjectPath, SchemaPath, catalog::FailurePoint, database::DatabaseState,
};

fn fixture() -> (Database, SchemaPath, ObjectPath) {
    let database = Database::builder().build();
    let schema = SchemaPath::regular("selene", "authority").unwrap();
    let graph = ObjectPath::regular("selene", "authority", "main").unwrap();
    database
        .catalog()
        .create_schema(&schema, CreatePolicy::Strict)
        .unwrap();
    database
        .catalog()
        .create_graph(&graph, None, CreatePolicy::Strict)
        .unwrap();
    (database, schema, graph)
}

fn ids(database: &Database, graph: &ObjectPath) -> (LowerSchemaId, LowerGraphId) {
    let snapshot = database.catalog().snapshot();
    let schema = snapshot.resolve_schema(&graph.schema_path()).unwrap();
    let graph = snapshot.resolve_graph(graph).unwrap();
    (
        LowerSchemaId::new(schema.id.get()).unwrap(),
        LowerGraphId::new(graph.id.get()).unwrap(),
    )
}

#[test]
fn graph_and_catalog_staging_stay_invisible_until_one_outer_store() {
    let (database, _, graph_path) = fixture();
    let (schema_id, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);
    let old = inner.state.load_full();

    inner.with_mutation_reservation(|reservation| {
        let mut draft = DatabaseDraft::new(&inner, &reservation);
        let instance = draft.pin_graph(graph_id).unwrap();
        let scratch = SharedGraph::from_graph(instance.graph.read().as_ref().clone());
        let mut txn = scratch.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let prepared = txn.prepare_unpublished(None, None).unwrap();
        draft.attach_prepared_graph(graph_id, prepared).unwrap();
        draft.materialize_prepared_graph(graph_id).unwrap();

        let mut catalog_txn = CatalogTransaction::new(&draft.state().catalog).unwrap();
        let schema_raw = draft.next.high_water.schema + 1;
        let added_schema = LowerSchemaId::new(schema_raw).unwrap();
        let descriptor = CatalogDescriptor::schema(
            added_schema,
            selene_catalog::CatalogName::regular("combined").unwrap(),
            draft.state().catalog.root_directory_id(),
            catalog_txn.generation(),
            CreationMetadata::new(catalog_txn.generation(), None),
        )
        .unwrap();
        catalog_txn.insert(descriptor).unwrap();
        draft.next.catalog = catalog_txn.build().unwrap();
        draft.next.high_water.schema = schema_raw;

        let still_old = inner.state.load_full();
        assert!(Arc::ptr_eq(&old, &still_old));
        assert!(
            still_old
                .catalog
                .descriptor(CatalogObjectId::Schema(added_schema))
                .is_none()
        );
        assert_eq!(still_old.graphs[&graph_id].graph.read().node_count(), 0);

        assert_eq!(
            inner.publish_database_draft(reservation, draft).unwrap(),
            AuthorityOutcome::Committed
        );
    });

    let published = inner.state.load_full();
    assert!(!Arc::ptr_eq(&old, &published));
    assert!(
        published
            .catalog
            .schema(&selene_catalog::CatalogName::regular("combined").unwrap())
            .is_some()
    );
    assert_eq!(published.graphs[&graph_id].graph.read().node_count(), 1);
    assert_eq!(schema_id.get(), 1);
}

#[test]
fn pre_store_cancellation_retains_exact_outer_arc_and_live_graph_ids() {
    let (database, _, graph_path) = fixture();
    let (_, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);
    let before = inner.state.load_full();
    let before_next = before.graphs[&graph_id].graph.read().meta.next_node_id;
    *inner.failure.lock() = Some(FailurePoint::BeforePublication);

    let outcome = inner.with_mutation_reservation(|reservation| {
        let mut draft = DatabaseDraft::new(&inner, &reservation);
        let instance = draft.pin_graph(graph_id).unwrap();
        let scratch = SharedGraph::from_graph(instance.graph.read().as_ref().clone());
        let mut txn = scratch.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let prepared = txn.prepare_unpublished(None, None).unwrap();
        draft.attach_prepared_graph(graph_id, prepared).unwrap();
        draft.materialize_prepared_graph(graph_id).unwrap();
        inner.publish_database_draft(reservation, draft).unwrap()
    });

    assert_eq!(outcome, AuthorityOutcome::Canceled);
    let after = inner.state.load_full();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(
        after.graphs[&graph_id].graph.read().meta.next_node_id,
        before_next
    );
}

#[test]
fn post_store_ack_failure_is_indeterminate_with_complete_state_visible() {
    let (database, _, graph_path) = fixture();
    let (_, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);
    let before = inner.state.load_full();
    *inner.failure.lock() = Some(FailurePoint::AfterPublicationAcknowledgement);

    let outcome = inner.with_mutation_reservation(|reservation| {
        let mut draft = DatabaseDraft::new(&inner, &reservation);
        let instance = draft.pin_graph(graph_id).unwrap();
        let scratch = SharedGraph::from_graph(instance.graph.read().as_ref().clone());
        let mut txn = scratch.begin_write();
        txn.mutator()
            .create_node(LabelSet::new(), PropertyMap::new())
            .unwrap();
        let prepared = txn.prepare_unpublished(None, None).unwrap();
        draft.attach_prepared_graph(graph_id, prepared).unwrap();
        draft.materialize_prepared_graph(graph_id).unwrap();
        inner.publish_database_draft(reservation, draft).unwrap()
    });

    assert_eq!(outcome, AuthorityOutcome::Indeterminate);
    let after = inner.state.load_full();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(after.graphs[&graph_id].graph.read().node_count(), 1);
}

#[test]
fn reservation_unlocks_while_unwinding() {
    let (database, _, _) = fixture();
    let inner = Arc::clone(&database.catalog().inner);
    let panic = catch_unwind(AssertUnwindSafe(|| {
        inner.with_mutation_reservation(|_| panic!("synthetic reservation panic"));
    }));
    assert!(panic.is_err());

    inner.with_mutation_reservation(|reservation| {
        let draft = DatabaseDraft::new(&inner, &reservation);
        assert_eq!(
            inner.publish_database_draft(reservation, draft).unwrap(),
            AuthorityOutcome::Committed
        );
    });
}

#[test]
fn stale_graph_generation_rejects_publication() {
    let (database, _, graph_path) = fixture();
    let (_, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);

    let error = inner.with_mutation_reservation(|reservation| {
        let mut draft = DatabaseDraft::new(&inner, &reservation);
        let instance = draft.pin_graph(graph_id).unwrap();
        instance.graph.begin_write().commit().unwrap();
        inner.publish_database_draft(reservation, draft)
    });
    assert!(error.is_err());
    assert_eq!(
        inner.state.load_full().graphs[&graph_id]
            .graph
            .read()
            .meta
            .generation,
        1
    );
    assert_eq!(CoreGraphId::new(graph_id.get()).get(), graph_id.get());
}

#[allow(dead_code)]
fn assert_database_state_is_send_static<T: Send + 'static>() {}

#[test]
fn stored_authority_types_are_send_and_static() {
    assert_database_state_is_send_static::<DatabaseDraft>();
    assert_database_state_is_send_static::<DatabaseState>();
}

#[test]
fn selected_maintenance_is_42n01_and_publishes_nothing() {
    let (database, _, graph_path) = fixture();
    let inner = Arc::clone(&database.catalog().inner);
    let session = database.session(&graph_path).unwrap();
    let before = inner.state.load_full();

    let error = session.execute("CALL selene.compact()").unwrap_err();

    assert_eq!(error.gqlstatus().unwrap().as_str(), "42N01");
    assert!(Arc::ptr_eq(&before, &inner.state.load_full()));
    assert_eq!(
        session.context().transaction_slot(),
        crate::TransactionSlotState::Vacant
    );
}

#[test]
fn direct_catalog_and_selected_write_share_one_reservation_without_loss() {
    let (database, _, graph_path) = fixture();
    let (_, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);
    let session = database.session(&graph_path).unwrap();
    let catalog = database.catalog();
    let start = Arc::new(std::sync::Barrier::new(3));
    let write_start = Arc::clone(&start);
    let writer = std::thread::spawn(move || {
        write_start.wait();
        session.execute("INSERT (:Concurrent) FINISH").unwrap();
    });
    let catalog_start = Arc::clone(&start);
    let catalog_writer = std::thread::spawn(move || {
        catalog_start.wait();
        catalog
            .create_schema(
                &SchemaPath::regular("selene", "alongside_write").unwrap(),
                CreatePolicy::Strict,
            )
            .unwrap();
    });
    start.wait();
    writer.join().unwrap();
    catalog_writer.join().unwrap();

    let state = inner.state.load_full();
    assert_eq!(state.graphs[&graph_id].graph.read().node_count(), 1);
    assert!(
        state
            .catalog
            .schema(&selene_catalog::CatalogName::regular("alongside_write").unwrap())
            .is_some()
    );
    assert_eq!(state.high_water.schema, 2);
}

#[test]
fn repeated_selected_writes_advance_ids_and_generation_once_each() {
    let (database, _, graph_path) = fixture();
    let (_, graph_id) = ids(&database, &graph_path);
    let inner = Arc::clone(&database.catalog().inner);
    let session = database.session(&graph_path).unwrap();
    let mut prior = inner.state.load_full();

    for expected in 1..=3 {
        session.execute("INSERT (:Repeated) FINISH").unwrap();
        let state = inner.state.load_full();
        assert!(!Arc::ptr_eq(&prior, &state));
        let graph = state.graphs[&graph_id].graph.read();
        assert_eq!(graph.meta.generation, expected);
        assert_eq!(graph.meta.next_node_id, expected + 1);
        for raw in 1..=expected {
            assert!(graph.is_node_alive(selene_core::NodeId::new(raw)));
        }
        drop(graph);
        prior = state;
    }
}

#[test]
fn selected_pre_store_failpoints_leave_exact_state_and_next_id() {
    for point in [
        FailurePoint::BeforeAuthorityPrepare,
        FailurePoint::BeforeAuthorityFlush,
        FailurePoint::BeforePublication,
    ] {
        let (database, _, graph_path) = fixture();
        let (_, graph_id) = ids(&database, &graph_path);
        let inner = Arc::clone(&database.catalog().inner);
        let session = database.session(&graph_path).unwrap();
        let before = inner.state.load_full();
        *inner.failure.lock() = Some(point);

        assert!(session.execute("INSERT (:Canceled) FINISH").is_err());

        let after = inner.state.load_full();
        assert!(Arc::ptr_eq(&before, &after));
        let graph = after.graphs[&graph_id].graph.read();
        assert_eq!(graph.meta.generation, 0);
        assert_eq!(graph.meta.next_node_id, 1);
        assert_eq!(graph.node_count(), 0);
    }
}
