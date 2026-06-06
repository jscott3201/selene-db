//! Integration coverage for selective vector-index maintenance.

use std::num::NonZeroUsize;

use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value, VectorValue};
use selene_graph::{SharedGraph, VectorIndexKind, VectorIndexMaintenancePolicy};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("test string fits DB string cap")
}

fn vector(components: &[f32]) -> Value {
    Value::Vector(VectorValue::new(components.to_vec()).expect("test vector is valid"))
}

fn props(property: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(property.clone(), value)]).expect("test property map is valid")
}

fn insert_vectors(
    shared: &SharedGraph,
    label: &DbString,
    property: &DbString,
    count: usize,
    offset: f32,
) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for value in 0..count {
        mutator
            .create_node(
                LabelSet::single(label.clone()),
                props(property, vector(&[offset + value as f32, 0.0])),
            )
            .expect("vector insert succeeds");
    }
    txn.commit().expect("vector seed commits");
}

#[test]
fn rebuild_recommended_vector_indexes_selects_only_drifted_ivf_indexes() {
    let shared = SharedGraph::new(GraphId::new(8_301));
    let property = db_string("embedding");
    let hot_label = db_string("vector.maintenance.hot");
    let cold_label = db_string("vector.maintenance.cold");
    insert_vectors(&shared, &hot_label, &property, 100, 0.0);
    insert_vectors(&shared, &cold_label, &property, 100, 10_000.0);

    shared
        .create_vector_index(
            hot_label.clone(),
            property.clone(),
            VectorIndexKind::IvfSquaredEuclidean,
            2,
        )
        .expect("hot ivf index creates");
    shared
        .create_vector_index(
            cold_label.clone(),
            property.clone(),
            VectorIndexKind::IvfSquaredEuclidean,
            2,
        )
        .expect("cold ivf index creates");

    insert_vectors(&shared, &hot_label, &property, 100, 20_000.0);
    insert_vectors(&shared, &cold_label, &property, 1, 30_000.0);

    let snapshot = shared.read();
    let hot_before = snapshot
        .vector_index_for(&hot_label, &property)
        .expect("hot index exists")
        .memory_usage();
    let cold_before = snapshot
        .vector_index_for(&cold_label, &property)
        .expect("cold index exists")
        .memory_usage();
    drop(snapshot);
    assert!(hot_before.ivf_rebuild_recommended());
    assert!(!cold_before.ivf_rebuild_recommended());

    let report = shared
        .rebuild_recommended_vector_indexes()
        .expect("recommended rebuild succeeds");

    assert_eq!(report.indexes_rebuilt, 1);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].label, hot_label);
    assert_eq!(report.entries[0].property, property);
    assert_eq!(report.entries[0].before, hot_before);
    assert_eq!(report.entries[0].after.ivf_pending_retrain_entries, 0);
    assert!(!report.entries[0].after.ivf_rebuild_recommended());

    let snapshot = shared.read();
    let hot_after = snapshot
        .vector_index_for(&hot_label, &property)
        .expect("hot index exists after rebuild")
        .memory_usage();
    let cold_after = snapshot
        .vector_index_for(&cold_label, &property)
        .expect("cold index exists after rebuild")
        .memory_usage();
    assert_eq!(hot_after, report.entries[0].after);
    assert_eq!(cold_after, cold_before);
    drop(snapshot);

    let noop = shared
        .rebuild_recommended_vector_indexes()
        .expect("second recommended rebuild succeeds");
    assert_eq!(noop.indexes_rebuilt, 0);
    assert!(noop.entries.is_empty());
}

#[test]
fn maintain_vector_indexes_caps_rebuilds_by_drift_pressure() {
    let shared = SharedGraph::new(GraphId::new(8_302));
    let property = db_string("embedding");
    let high_label = db_string("vector.maintenance.high");
    let low_label = db_string("vector.maintenance.low");
    let cold_label = db_string("vector.maintenance.cold");
    for (label, offset) in [
        (&high_label, 0.0),
        (&low_label, 10_000.0),
        (&cold_label, 20_000.0),
    ] {
        insert_vectors(&shared, label, &property, 100, offset);
        shared
            .create_vector_index(
                label.clone(),
                property.clone(),
                VectorIndexKind::IvfSquaredEuclidean,
                2,
            )
            .expect("ivf index creates");
    }
    insert_vectors(&shared, &high_label, &property, 200, 30_000.0);
    insert_vectors(&shared, &low_label, &property, 100, 40_000.0);
    insert_vectors(&shared, &cold_label, &property, 1, 50_000.0);

    let one_at_a_time = VectorIndexMaintenancePolicy::recommended()
        .with_max_indexes_per_run(NonZeroUsize::new(1).expect("one is non-zero"));

    let first = shared
        .maintain_vector_indexes(one_at_a_time)
        .expect("first capped maintenance succeeds");
    assert_eq!(first.indexes_rebuilt, 1);
    assert_eq!(first.entries[0].label, high_label);
    assert!(first.entries[0].before.ivf_rebuild_recommended());
    assert!(!first.entries[0].after.ivf_rebuild_recommended());

    let snapshot = shared.read();
    assert!(
        !snapshot
            .vector_index_for(&high_label, &property)
            .expect("high index remains registered")
            .memory_usage()
            .ivf_rebuild_recommended()
    );
    assert!(
        snapshot
            .vector_index_for(&low_label, &property)
            .expect("low index remains registered")
            .memory_usage()
            .ivf_rebuild_recommended()
    );
    assert!(
        !snapshot
            .vector_index_for(&cold_label, &property)
            .expect("cold index remains registered")
            .memory_usage()
            .ivf_rebuild_recommended()
    );
    drop(snapshot);

    let second = shared
        .maintain_vector_indexes(one_at_a_time)
        .expect("second capped maintenance succeeds");
    assert_eq!(second.indexes_rebuilt, 1);
    assert_eq!(second.entries[0].label, low_label);
    assert!(second.entries[0].before.ivf_rebuild_recommended());
    assert!(!second.entries[0].after.ivf_rebuild_recommended());
}
