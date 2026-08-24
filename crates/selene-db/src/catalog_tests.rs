use std::{
    collections::BTreeSet,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use selene_catalog::{CatalogObjectId, CatalogObjectKind, GraphId as LowerGraphId};

use super::*;
use crate::{
    CreatePolicy, Database, DropPolicy, ErrorKind, ExecutionOutcome, GqlStatus, ObjectPath,
    SchemaPath,
};

fn schema(name: &str) -> SchemaPath {
    SchemaPath::regular("selene", name).unwrap()
}

fn graph(schema: &str, name: &str) -> ObjectPath {
    ObjectPath::regular("selene", schema, name).unwrap()
}

fn assert_outer_complete(state: &DatabaseState) {
    let descriptor_graphs = state
        .catalog
        .descriptors()
        .filter_map(|descriptor| match descriptor.id() {
            CatalogObjectId::Graph(id) => Some(id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        descriptor_graphs,
        state.graphs.keys().copied().collect::<BTreeSet<_>>()
    );
    let descriptor_types = state
        .catalog
        .descriptors()
        .filter_map(|descriptor| match descriptor.id() {
            CatalogObjectId::GraphType(id) => Some(id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        descriptor_types,
        state.graph_types.keys().copied().collect::<BTreeSet<_>>()
    );
    for descriptor in state.catalog.descriptors() {
        if descriptor.kind() == CatalogObjectKind::Graph {
            assert!(matches!(descriptor.id(), CatalogObjectId::Graph(_)));
        }
    }
}

fn assert_failure_preserves_outer_state<T: std::fmt::Debug>(
    catalog: &Catalog,
    point: FailurePoint,
    operation: impl FnOnce() -> Result<T>,
) {
    let before = catalog.inner.state.load_full();
    *catalog.inner.failure.lock() = Some(point);
    let error = operation().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CatalogInvariant);
    let after = catalog.inner.state.load_full();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(before.catalog.generation(), after.catalog.generation());
    assert_eq!(before.graphs.len(), after.graphs.len());
    assert_eq!(before.graph_types.len(), after.graph_types.len());
    assert_eq!(before.high_water, after.high_water);
    assert_outer_complete(&after);
}

#[test]
fn injected_staging_failures_cover_each_lifecycle_object_and_drop() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema_path = schema("failures");
    assert_failure_preserves_outer_state(&catalog, FailurePoint::AfterDescriptorStaging, || {
        catalog.create_schema(&schema_path, CreatePolicy::Strict)
    });
    catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap();
    let type_path = graph("failures", "type");
    assert_failure_preserves_outer_state(&catalog, FailurePoint::AfterDescriptorStaging, || {
        catalog.create_graph_type(&type_path, test_graph_type(), CreatePolicy::Strict)
    });
    catalog
        .create_graph_type(&type_path, test_graph_type(), CreatePolicy::Strict)
        .unwrap();
    let reserved_graph_id = catalog.inner.state.load().high_water.graph + 1;
    for point in [
        FailurePoint::AfterDescriptorStaging,
        FailurePoint::AfterGraphConstruction,
    ] {
        assert_failure_preserves_outer_state(&catalog, point, || {
            catalog.create_graph(
                &graph("failures", "g"),
                Some(&type_path),
                CreatePolicy::Strict,
            )
        });
    }
    let CreateOutcome::Created(created) = catalog
        .create_graph(
            &graph("failures", "g"),
            Some(&type_path),
            CreatePolicy::Strict,
        )
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(created.id.get(), reserved_graph_id);

    let drop_path = graph("failures", "drop_target");
    catalog
        .create_graph(&drop_path, None, CreatePolicy::Strict)
        .unwrap();
    assert_failure_preserves_outer_state(&catalog, FailurePoint::AfterDescriptorStaging, || {
        catalog.drop_graph(&drop_path, DropPolicy::Strict)
    });
    assert!(catalog.snapshot().resolve_graph(&drop_path).is_ok());
}

/// The replace path shares every failure point with create; each one must
/// leave the old graph registered under its original identity and reserve
/// no identity for the replacement that never published.
#[test]
fn injected_failures_on_the_replace_path_keep_the_old_graph() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("replace"), CreatePolicy::Strict)
        .unwrap();
    let path = graph("replace", "g");
    let CreateOutcome::Created(original) = catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let handle = catalog.open_graph(&path).unwrap();
    let reserved = catalog.inner.state.load().high_water.graph + 1;
    for point in [
        FailurePoint::AfterDescriptorStaging,
        FailurePoint::AfterGraphConstruction,
        FailurePoint::BeforePublication,
    ] {
        assert_failure_preserves_outer_state(&catalog, point, || {
            catalog.create_graph(&path, None, CreatePolicy::OrReplace)
        });
        assert_eq!(catalog.snapshot().resolve_graph(&path).unwrap(), original);
        handle.execute("RETURN 1").unwrap();
    }
    let CreateOutcome::Replaced { dropped, created } = catalog
        .create_graph(&path, None, CreatePolicy::OrReplace)
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(dropped, original);
    assert_eq!(created.id.get(), reserved);
    assert_eq!(
        handle.execute("RETURN 1").unwrap_err().kind(),
        ErrorKind::StaleGraphHandle
    );
    assert_outer_complete(&catalog.inner.state.load_full());
}

fn test_graph_type() -> crate::GraphTypeDefinition {
    crate::GraphTypeDefinition::builder()
        .with_node_type(
            crate::NodeTypeDefinition::new(
                crate::PathSegment::regular("PersonType").unwrap(),
                vec![crate::PathSegment::regular("Person").unwrap()],
            )
            .unwrap(),
        )
        .build()
        .unwrap()
}

#[test]
fn each_create_path_can_fail_at_the_final_prepublication_boundary() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let schema_path = schema("prepublish");
    assert_failure_preserves_outer_state(&catalog, FailurePoint::BeforePublication, || {
        catalog.create_schema(&schema_path, CreatePolicy::Strict)
    });
    catalog
        .create_schema(&schema_path, CreatePolicy::Strict)
        .unwrap();

    let type_path = graph("prepublish", "type");
    assert_failure_preserves_outer_state(&catalog, FailurePoint::BeforePublication, || {
        catalog.create_graph_type(&type_path, test_graph_type(), CreatePolicy::Strict)
    });
    catalog
        .create_graph_type(&type_path, test_graph_type(), CreatePolicy::Strict)
        .unwrap();

    let graph_path = graph("prepublish", "graph");
    assert_failure_preserves_outer_state(&catalog, FailurePoint::BeforePublication, || {
        catalog.create_graph(&graph_path, Some(&type_path), CreatePolicy::Strict)
    });
}

#[test]
fn retained_snapshot_keeps_the_complete_old_runtime_publication() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("retained"), CreatePolicy::Strict)
        .unwrap();
    let path = graph("retained", "g");
    let CreateOutcome::Created(descriptor) = catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    else {
        unreachable!()
    };
    let retained = catalog.snapshot();
    let id = LowerGraphId::new(descriptor.id.get()).unwrap();
    let instance = retained.state.graphs.get(&id).unwrap().clone();

    catalog.drop_graph(&path, DropPolicy::Strict).unwrap();

    assert!(retained.state.graphs.contains_key(&id));
    assert!(Arc::ptr_eq(
        retained.state.graphs.get(&id).unwrap(),
        &instance
    ));
    assert!(!catalog.snapshot().state.graphs.contains_key(&id));
    assert!(retained.resolve_graph(&path).is_ok());
    assert!(catalog.snapshot().resolve_graph(&path).is_err());
}

#[test]
fn concurrent_readers_observe_only_complete_outer_publications() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("concurrent"), CreatePolicy::Strict)
        .unwrap();
    let start = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let reader_inner = Arc::clone(&catalog.inner);
    let reader_start = Arc::clone(&start);
    let reader_finished = Arc::clone(&finished);
    let reader = thread::spawn(move || {
        reader_start.wait();
        while !reader_finished.load(Ordering::Acquire) {
            assert_outer_complete(&reader_inner.state.load_full());
        }
        assert_outer_complete(&reader_inner.state.load_full());
    });

    start.wait();
    for index in 0..8 {
        catalog
            .create_graph(
                &graph("concurrent", &format!("g{index}")),
                None,
                CreatePolicy::Strict,
            )
            .unwrap();
    }
    finished.store(true, Ordering::Release);
    reader.join().unwrap();
}

#[test]
fn concurrent_readers_observe_only_complete_publications_from_gql_ddl() {
    let database = Database::builder().build();
    let session = database.session();
    let start = Arc::new(Barrier::new(2));
    let finished = Arc::new(AtomicBool::new(false));
    let reader_inner = Arc::clone(&database.catalog().inner);
    let reader_start = Arc::clone(&start);
    let reader_finished = Arc::clone(&finished);
    let reader = thread::spawn(move || {
        reader_start.wait();
        while !reader_finished.load(Ordering::Acquire) {
            assert_outer_complete(&reader_inner.state.load_full());
        }
        assert_outer_complete(&reader_inner.state.load_full());
    });

    start.wait();
    session.execute("CREATE SCHEMA /concurrent").unwrap();
    for index in 0..8 {
        session
            .execute(&format!("CREATE GRAPH /concurrent/g{index} ANY"))
            .unwrap();
    }
    // Each replacement removes one descriptor/instance pair and adds another
    // in the same swap; a reader must never see the pair half-applied.
    for index in 0..8 {
        session
            .execute(&format!("CREATE OR REPLACE GRAPH /concurrent/g{index} ANY"))
            .unwrap();
    }
    for index in 0..8 {
        session
            .execute(&format!("DROP GRAPH /concurrent/g{index}"))
            .unwrap();
    }
    session.execute("DROP SCHEMA /concurrent").unwrap();
    finished.store(true, Ordering::Release);
    reader.join().unwrap();
}

/// Every database-catalog statement dispatches after the parse lease is
/// released. The test-only lease accounting in `lock_lifecycle_writer` panics
/// if any `Catalog` mutation runs under a same-thread graph request lease, so
/// this test passing proves hard constraint A for each statement kind,
/// including the bootstrap `DROP GRAPH` bridge.
#[test]
fn gql_catalog_statements_dispatch_outside_the_graph_request_lease() {
    let database = Database::builder().build();
    let session = database.session();
    let omitted = ExecutionOutcome::OmittedResult {
        status: GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
    };
    assert_eq!(session.execute("CREATE SCHEMA /lease").unwrap(), omitted);
    assert_eq!(
        session.execute("CREATE GRAPH /lease/g ANY").unwrap(),
        omitted
    );
    assert_eq!(session.execute("CREATE GRAPH h ANY").unwrap(), omitted);
    assert_eq!(
        session.execute("CREATE OR REPLACE GRAPH h ANY").unwrap(),
        omitted
    );
    assert_eq!(session.execute("DROP GRAPH h").unwrap(), omitted);
    assert_eq!(session.execute("DROP GRAPH /lease/g").unwrap(), omitted);
    assert_eq!(session.execute("DROP SCHEMA /lease").unwrap(), omitted);
    assert!(matches!(
        session.execute("DROP GRAPH default").unwrap(),
        ExecutionOutcome::Written(_)
    ));
    assert_eq!(crate::database::GraphRequestDepth::current(), 0);
}

#[test]
#[should_panic(expected = "catalog lifecycle entered under a same-thread graph request lease")]
fn lease_accounting_catches_catalog_mutation_under_a_request_lease() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let inner = Arc::clone(&catalog.inner);
    let path = graph("public", "default");
    inner
        .with_graph_request(bootstrap_graph_id(), &path, |_| {
            catalog.create_schema(&schema("under_lease"), CreatePolicy::Strict)
        })
        .unwrap();
}

#[test]
fn serialized_concurrent_writers_publish_unique_ids_without_lost_updates() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    let start = Arc::new(Barrier::new(9));
    let writers = (0..8)
        .map(|index| {
            let catalog = catalog.clone();
            let start = Arc::clone(&start);
            thread::spawn(move || {
                start.wait();
                catalog
                    .create_schema(&schema(&format!("writer{index}")), CreatePolicy::Strict)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    let ids = writers
        .into_iter()
        .map(|writer| match writer.join().unwrap() {
            CreateOutcome::Created(descriptor) => descriptor.id,
            CreateOutcome::AlreadyExists(_) | CreateOutcome::Replaced { .. } => unreachable!(),
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 8);
    assert_eq!(catalog.snapshot().schemas().unwrap().len(), 9);
}

#[test]
fn active_request_finishes_before_drop_and_drop_observes_its_write() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("active"), CreatePolicy::Strict)
        .unwrap();
    let path = graph("active", "g");
    let created = match catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    {
        CreateOutcome::Created(descriptor) => descriptor,
        CreateOutcome::AlreadyExists(_) | CreateOutcome::Replaced { .. } => unreachable!(),
    };
    let id = LowerGraphId::new(created.id.get()).unwrap();
    let (leased_tx, leased_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let request_inner = Arc::clone(&catalog.inner);
    let request_path = path.clone();
    let request_release = Arc::clone(&release);
    let request = thread::spawn(move || {
        request_inner.with_graph_request(id, &request_path, |shared| {
            leased_tx.send(()).unwrap();
            request_release.wait();
            let mut session = selene_gql::Session::new(shared);
            session
                .execute_source_named_graph("INSERT (:Person)", &request_inner.procedures)
                .map_err(Error::from_engine)
        })
    });
    leased_rx.recv().unwrap();
    let (drop_blocked_tx, drop_blocked_rx) = mpsc::channel();
    *catalog.inner.drop_blocked.lock() = Some(drop_blocked_tx);
    let drop_catalog = catalog.clone();
    let drop_path = path.clone();
    let dropper = thread::spawn(move || drop_catalog.drop_graph(&drop_path, DropPolicy::Strict));
    drop_blocked_rx.recv().unwrap();
    release.wait();
    request.join().unwrap().unwrap();
    let error = dropper.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CatalogRestrictViolation);
    assert!(error.message().contains("1 nodes, 0 edges"));
}

/// Replacement waits for an active request lease exactly as drop does, and
/// then applies RESTRICT to the write that request committed.
#[test]
fn active_request_finishes_before_replace_and_replace_observes_its_write() {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("active"), CreatePolicy::Strict)
        .unwrap();
    let path = graph("active", "g");
    let created = match catalog
        .create_graph(&path, None, CreatePolicy::Strict)
        .unwrap()
    {
        CreateOutcome::Created(descriptor) => descriptor,
        CreateOutcome::AlreadyExists(_) | CreateOutcome::Replaced { .. } => unreachable!(),
    };
    let id = LowerGraphId::new(created.id.get()).unwrap();
    let (leased_tx, leased_rx) = mpsc::channel();
    let release = Arc::new(Barrier::new(2));
    let request_inner = Arc::clone(&catalog.inner);
    let request_path = path.clone();
    let request_release = Arc::clone(&release);
    let request = thread::spawn(move || {
        request_inner.with_graph_request(id, &request_path, |shared| {
            leased_tx.send(()).unwrap();
            request_release.wait();
            let mut session = selene_gql::Session::new(shared);
            session
                .execute_source_named_graph("INSERT (:Person)", &request_inner.procedures)
                .map_err(Error::from_engine)
        })
    });
    leased_rx.recv().unwrap();
    let (blocked_tx, blocked_rx) = mpsc::channel();
    *catalog.inner.drop_blocked.lock() = Some(blocked_tx);
    let replace_catalog = catalog.clone();
    let replace_path = path.clone();
    let replacer = thread::spawn(move || {
        replace_catalog.create_graph(&replace_path, None, CreatePolicy::OrReplace)
    });
    blocked_rx.recv().unwrap();
    release.wait();
    request.join().unwrap().unwrap();
    let error = replacer.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::CatalogRestrictViolation);
    assert!(error.message().contains("1 nodes, 0 edges"));
    assert_eq!(catalog.snapshot().resolve_graph(&path).unwrap(), created);
}
