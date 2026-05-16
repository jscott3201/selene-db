//! Default-index transparency tests for BRIEF-109 PR1 registries.

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderTag, SubTag};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswIndexRegistry, HnswProvider, IvfConfig, IvfIndexRegistry,
    IvfProvider, PqParams, VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};

#[test]
fn registry_construction_seeds_default_entry() {
    let hnsw = HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds");
    assert_eq!(hnsw.provider_tag(), ProviderTag(*b"VECT"));
    assert!(hnsw.get("default").is_some());
    assert!(hnsw.get("episodes").is_none());

    let ivf = IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds");
    assert_eq!(ivf.provider_tag(), ProviderTag(*b"IVFP"));
    assert!(ivf.get("default").is_some());
    assert!(ivf.get("episodes").is_none());
}

#[test]
fn hnsw_registry_snapshot_and_wal_replay_match_singleton() {
    let direct = HnswProvider::new(hnsw_config()).expect("direct provider builds");
    let registry = HnswIndexRegistry::new(hnsw_config()).expect("registry builds");
    let change = hnsw_change(1, [1.0, 0.0, 0.0, 0.0]);

    direct
        .on_change(&change)
        .expect("direct WAL replay applies");
    registry
        .on_change(&change)
        .expect("registry WAL replay applies");

    assert_eq!(
        direct
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("direct search succeeds"),
        registry
            .get("default")
            .expect("default provider exists")
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("registry search succeeds")
    );
    assert_eq!(hnsw_sections(&direct), hnsw_sections(&registry));
}

#[test]
fn ivf_registry_snapshot_and_wal_replay_match_singleton() {
    let direct = IvfProvider::new(ivf_config()).expect("direct provider builds");
    let registry = IvfIndexRegistry::new(ivf_config()).expect("registry builds");
    let change = ivf_change(1, [1.0, 0.0, 0.0, 0.0]);

    direct
        .on_change(&change)
        .expect("direct WAL replay applies");
    registry
        .on_change(&change)
        .expect("registry WAL replay applies");

    assert_eq!(
        direct.ivf_stats().expect("direct stats succeed"),
        registry
            .get("default")
            .expect("default provider exists")
            .ivf_stats()
            .expect("registry stats succeed")
    );
    assert_eq!(ivf_sections(&direct), ivf_sections(&registry));
}

fn hnsw_config() -> HnswConfig {
    HnswConfig::new(4).expect("HNSW config is valid")
}

fn ivf_config() -> IvfConfig {
    IvfConfig::with_params(
        4,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .expect("IVF config is valid")
}

fn hnsw_change(raw: u64, vector: [f32; 4]) -> Change {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(raw),
        vector: vector.to_vec(),
        max_layer: 0,
    }
    .encode()
    .expect("VECU payload encodes");
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").expect("provider name interns"),
        payload: Arc::from(payload.into_boxed_slice()),
    }
}

fn ivf_change(raw: u64, vector: [f32; 4]) -> Change {
    let payload = VectorIvfUpsertV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(raw),
        vector: vector.to_vec(),
    }
    .encode()
    .expect("VIVF payload encodes");
    Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").expect("provider name interns"),
        payload: Arc::from(payload.into_boxed_slice()),
    }
}

fn hnsw_sections(provider: &dyn IndexProvider) -> Vec<Vec<u8>> {
    [SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")]
        .into_iter()
        .map(|sub_tag| {
            provider
                .write_section(sub_tag)
                .expect("HNSW section writes")
        })
        .collect()
}

fn ivf_sections(provider: &dyn IndexProvider) -> Vec<Vec<u8>> {
    [SubTag(*b"CQNT"), SubTag(*b"IPQB"), SubTag(*b"POST")]
        .into_iter()
        .map(|sub_tag| provider.write_section(sub_tag).expect("IVF section writes"))
        .collect()
}
