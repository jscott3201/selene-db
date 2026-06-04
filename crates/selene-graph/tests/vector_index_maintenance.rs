//! Integration coverage for selective vector-index maintenance.

use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, VectorValue, intern};
use selene_graph::{SharedGraph, VectorIndexKind};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn vector(components: &[f32]) -> Value {
    Value::Vector(VectorValue::new(components.to_vec()).expect("test vector is valid"))
}

fn props(property: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(property.clone(), value)]).expect("test property map is valid")
}

fn insert_vectors(shared: &SharedGraph, label: &IStr, property: &IStr, count: usize, offset: f32) {
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
    let property = istr("embedding");
    let hot_label = istr("vector.maintenance.hot");
    let cold_label = istr("vector.maintenance.cold");
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
