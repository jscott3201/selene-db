use selene_core::{DbString, GraphId, IvfIndexConfig, LabelSet, PropertyMap, Value, VectorValue};

use crate::{GraphError, SharedGraph, VectorIndexConfig, VectorIndexKind};

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).unwrap()
}

fn vector(components: &[f32]) -> Value {
    Value::Vector(VectorValue::new(components.to_vec()).unwrap())
}

fn props(pairs: impl IntoIterator<Item = (DbString, Value)>) -> PropertyMap {
    PropertyMap::from_pairs(pairs).unwrap()
}

#[test]
fn explicit_ivf_config_controls_centroid_count() {
    let shared = SharedGraph::new(GraphId::new(8110));
    let label = db_string("vector.index.ivf.config");
    let property = db_string("embedding");
    let config = IvfIndexConfig::new(4);
    {
        let mut txn = shared.begin_write();
        let mut mutator = txn.mutator();
        for idx in 0..8 {
            mutator
                .create_node(
                    LabelSet::single(label.clone()),
                    props([(property.clone(), vector(&[idx as f32, 1.0]))]),
                )
                .unwrap();
        }
        txn.commit().unwrap();
    }

    shared
        .create_vector_index_named_with_configs(
            label.clone(),
            property.clone(),
            VectorIndexKind::IvfCosine,
            2,
            None,
            VectorIndexConfig::ivf(config),
        )
        .unwrap();

    let snapshot = shared.read();
    let index = snapshot.vector_index_for(&label, &property).unwrap();
    assert_eq!(index.ivf_config(), Some(config));
    assert_eq!(
        index.memory_usage().ivf_centroids,
        usize::from(config.target_centroids)
    );
    assert_eq!(
        index.memory_usage().ivf_list_count,
        usize::from(config.target_centroids)
    );
}

#[test]
fn ivf_config_is_rejected_for_non_ivf_or_invalid_shapes() {
    let shared = SharedGraph::new(GraphId::new(8111));
    let label = db_string("vector.index.ivf.config.reject");
    let property = db_string("embedding");

    let hnsw_err = shared
        .create_vector_index_named_with_configs(
            label.clone(),
            property.clone(),
            VectorIndexKind::HnswCosine,
            2,
            None,
            VectorIndexConfig::ivf(IvfIndexConfig::new(4)),
        )
        .unwrap_err();
    assert!(matches!(
        hnsw_err,
        GraphError::VectorIndexInvalidIvfConfig {
            target_centroids: 4,
            reason: "only IVF vector indexes accept IVF config",
        }
    ));

    let invalid_err = shared
        .create_vector_index_named_with_configs(
            label,
            property,
            VectorIndexKind::IvfSquaredEuclidean,
            2,
            None,
            VectorIndexConfig::ivf(IvfIndexConfig::new(0)),
        )
        .unwrap_err();
    assert!(matches!(
        invalid_err,
        GraphError::VectorIndexInvalidIvfConfig {
            target_centroids: 0,
            reason: "target_centroids must be greater than zero",
        }
    ));
}
