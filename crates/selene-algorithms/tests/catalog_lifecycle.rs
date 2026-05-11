//! Integration tests for `ProjectionCatalog` covering the §H test bar.

use std::thread;

use selene_algorithms::{AlgorithmsError, ProjectionCatalog, ProjectionConfig};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_graph::SharedGraph;

fn istr(name: &str) -> IStr {
    intern(name).unwrap()
}

/// Two-Person-one-Company fixture: matches BRIEF-50's `fixture_small` shape.
fn fixture_small() -> (SharedGraph, [NodeId; 3]) {
    let shared = SharedGraph::new(GraphId::new(1));
    let person = istr("Person");
    let company = istr("Company");
    let friend = istr("friend");

    let mut txn = shared.begin_write();
    let n1 = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    let n2 = txn
        .mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    let n3 = txn
        .mutator()
        .create_node(LabelSet::single(company), PropertyMap::new())
        .unwrap();
    txn.mutator()
        .create_edge(friend, n1, n2, PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();
    (shared, [n1, n2, n3])
}

fn social_config() -> ProjectionConfig {
    ProjectionConfig {
        name: "social".to_string(),
        node_labels: vec![],
        edge_labels: vec![],
        weight_property: None,
    }
}

#[test]
fn catalog_project_and_get() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    let (n, e) = catalog.project(&snapshot, &social_config(), None).unwrap();
    assert_eq!(n, 3);
    assert_eq!(e, 1);
    assert!(catalog.contains("social"));
    assert_eq!(catalog.len(), 1);
    assert!(!catalog.is_empty());

    let proj_ref = catalog.get("social").expect("entry exists");
    assert_eq!(proj_ref.node_count(), 3);
    assert_eq!(proj_ref.edge_count(), 1);
}

#[test]
fn ensure_fresh_no_op_when_generation_matches() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    catalog.project(&snapshot, &social_config(), None).unwrap();

    let gen_before = catalog.get("social").unwrap().generation();
    catalog
        .ensure_fresh(&snapshot, "social")
        .expect("ensure_fresh succeeds on matching generation");
    let gen_after = catalog.get("social").unwrap().generation();
    assert_eq!(gen_before, gen_after);
    // node_count is unchanged because the underlying snapshot is unchanged.
    assert_eq!(catalog.get("social").unwrap().node_count(), 3);
}

#[test]
fn ensure_fresh_rebuilds_on_stale_generation() {
    let (shared, _) = fixture_small();
    let snapshot_v1 = shared.read();
    let catalog = ProjectionCatalog::new();
    catalog
        .project(&snapshot_v1, &social_config(), None)
        .unwrap();
    let gen_v1 = catalog.get("social").unwrap().generation();
    drop(snapshot_v1);

    // Mutate the graph: append a third Person.
    let mut txn = shared.begin_write();
    let person = istr("Person");
    txn.mutator()
        .create_node(LabelSet::single(person), PropertyMap::new())
        .unwrap();
    txn.commit().unwrap();

    let snapshot_v2 = shared.read();
    let gen_v2 = snapshot_v2.meta.generation;
    assert!(
        gen_v2 > gen_v1,
        "graph generation must advance after commit"
    );

    catalog
        .ensure_fresh(&snapshot_v2, "social")
        .expect("ensure_fresh succeeds and rebuilds");

    let proj_ref = catalog.get("social").unwrap();
    assert_eq!(
        proj_ref.generation(),
        gen_v2,
        "projection generation matches the fresh snapshot after rebuild"
    );
    assert_eq!(
        proj_ref.node_count(),
        4,
        "rebuilt projection sees the new node"
    );
}

#[test]
fn ensure_fresh_missing_name_errors() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();

    let err = catalog
        .ensure_fresh(&snapshot, "nonexistent")
        .expect_err("ensure_fresh on missing name must error");
    match err {
        AlgorithmsError::NoSuchProjection { name } => {
            assert_eq!(name, "nonexistent");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[test]
fn get_missing_name_returns_none() {
    let catalog = ProjectionCatalog::new();
    assert!(catalog.get("nonexistent").is_none());
    assert!(!catalog.contains("nonexistent"));
    assert_eq!(catalog.len(), 0);
    assert!(catalog.is_empty());
}

#[test]
fn drop_projection_removes_entry() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    catalog.project(&snapshot, &social_config(), None).unwrap();
    assert!(catalog.contains("social"));

    assert!(catalog.drop_projection("social"));
    assert!(!catalog.contains("social"));
    assert!(catalog.get("social").is_none());
    let err = catalog
        .ensure_fresh(&snapshot, "social")
        .expect_err("ensure_fresh on dropped name errors");
    matches!(err, AlgorithmsError::NoSuchProjection { .. })
        .then_some(())
        .expect("error is NoSuchProjection");

    // Subsequent drop returns false.
    assert!(!catalog.drop_projection("social"));
}

#[test]
fn project_overwrites_existing_name() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();

    // First projection: all nodes + all edges.
    catalog.project(&snapshot, &social_config(), None).unwrap();
    assert_eq!(catalog.get("social").unwrap().node_count(), 3);

    // Same name, different shape: only Person-labelled nodes.
    let person_only = ProjectionConfig {
        name: "social".to_string(),
        node_labels: vec![istr("Person")],
        edge_labels: vec![],
        weight_property: None,
    };
    let (n, _) = catalog.project(&snapshot, &person_only, None).unwrap();
    assert_eq!(n, 2, "Person-filter yields 2 nodes");
    assert_eq!(catalog.get("social").unwrap().node_count(), 2);
    assert_eq!(catalog.len(), 1, "overwrite does not grow the catalog");

    // Sanity: names() reflects the single registered projection.
    let names = catalog.names();
    assert_eq!(names, vec!["social".to_string()]);
}

#[test]
fn concurrent_reads_share_projection() {
    let (shared, _) = fixture_small();
    let snapshot = shared.read();
    let catalog = ProjectionCatalog::new();
    catalog.project(&snapshot, &social_config(), None).unwrap();

    const READER_COUNT: usize = 8;
    const ITERATIONS: usize = 64;

    thread::scope(|scope| {
        let handles: Vec<_> = (0..READER_COUNT)
            .map(|_| {
                scope.spawn(|| {
                    for _ in 0..ITERATIONS {
                        let proj_ref = catalog
                            .get("social")
                            .expect("entry exists for all concurrent readers");
                        assert_eq!(proj_ref.node_count(), 3);
                        assert_eq!(proj_ref.edge_count(), 1);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("reader thread panicked");
        }
    });

    // After all readers join, the catalog still has the single entry.
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog.get("social").unwrap().node_count(), 3);
}
