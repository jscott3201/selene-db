//! Default-index transparency tests for BRIEF-109 registries.

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, ProviderTag, SubTag};
use selene_vector::{
    Catalog, DistanceMetric, HnswConfig, HnswIndexRegistry, HnswProvider, IvfConfig,
    IvfIndexRegistry, IvfProvider, PqParams, VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
    encode_hnsw_config, encode_ivf_config,
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
    let registry_sections = hnsw_sections(&registry);
    assert_v1_wrapped(&registry_sections);
    let recovered = HnswIndexRegistry::new(hnsw_config()).expect("recovery registry builds");
    read_hnsw_sections(&recovered, &registry_sections);
    assert_eq!(
        direct
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("direct search succeeds"),
        recovered
            .get("default")
            .expect("default provider exists")
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None, None)
            .expect("recovered search succeeds")
    );
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
    let registry_sections = ivf_sections(&registry);
    assert_v1_wrapped(&registry_sections);
    let recovered = IvfIndexRegistry::new(ivf_config()).expect("recovery registry builds");
    read_ivf_sections(&recovered, &registry_sections);
    assert_eq!(
        direct.ivf_stats().expect("direct stats succeed"),
        recovered
            .get("default")
            .expect("default provider exists")
            .ivf_stats()
            .expect("recovered stats succeed")
    );
}

#[test]
fn hnsw_recovery_rejects_vecs_missing_entry() {
    let hnsw = Arc::new(HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds"));
    let ivf = Arc::new(IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds"));
    Catalog::from_registries(Arc::clone(&hnsw), ivf)
        .create_hnsw_index("episodes", hnsw_config())
        .expect("named HNSW creates");
    let grph = hnsw
        .write_section(SubTag(*b"GRPH"))
        .expect("multi-entry GRPH writes");
    let default_hnsw = HnswIndexRegistry::new(hnsw_config()).expect("default-only registry builds");
    let default_sections = hnsw_sections(&default_hnsw);
    let vecs = default_sections[1].clone();
    let recovered = HnswIndexRegistry::new(hnsw_config()).expect("recovery registry builds");

    recovered
        .read_section(SubTag(*b"GRPH"), &grph)
        .expect("GRPH stages entries");
    let err = recovered
        .read_section(SubTag(*b"VECS"), &vecs)
        .expect_err("VECS missing named entry rejected");

    assert_invalid_payload_contains(err, "VECS wrapper missing entry 'episodes'");
}

#[test]
fn ivf_recovery_rejects_ipqb_missing_entry() {
    let hnsw = Arc::new(HnswIndexRegistry::new(hnsw_config()).expect("HNSW registry builds"));
    let ivf = Arc::new(IvfIndexRegistry::new(ivf_config()).expect("IVF registry builds"));
    Catalog::from_registries(hnsw, Arc::clone(&ivf))
        .create_ivf_index("staged", ivf_config())
        .expect("named IVF creates");
    let cqnt = ivf
        .write_section(SubTag(*b"CQNT"))
        .expect("multi-entry CQNT writes");
    let default_ivf = IvfIndexRegistry::new(ivf_config()).expect("default-only registry builds");
    let default_sections = ivf_sections(&default_ivf);
    let ipqb = default_sections[1].clone();
    let recovered = IvfIndexRegistry::new(ivf_config()).expect("recovery registry builds");

    recovered
        .read_section(SubTag(*b"CQNT"), &cqnt)
        .expect("CQNT stages entries");
    let err = recovered
        .read_section(SubTag(*b"IPQB"), &ipqb)
        .expect_err("IPQB missing named entry rejected");

    assert_invalid_payload_contains(err, "IPQB wrapper missing entry 'staged'");
}

#[test]
fn hnsw_recovery_rejects_wrapper_missing_default() {
    let grph = hnsw_wrapper(SubTag(*b"GRPH"), &["a", "b"]);
    let recovered = HnswIndexRegistry::new(hnsw_config()).expect("recovery registry builds");

    let err = recovered
        .read_section(SubTag(*b"GRPH"), &grph)
        .expect_err("GRPH without default rejected");

    assert_invalid_payload_contains(err, "VECT GRPH wrapper missing 'default' entry");
}

#[test]
fn ivf_recovery_rejects_wrapper_missing_default() {
    let cqnt = ivf_wrapper(SubTag(*b"CQNT"), &["a", "b"]);
    let recovered = IvfIndexRegistry::new(ivf_config()).expect("recovery registry builds");

    let err = recovered
        .read_section(SubTag(*b"CQNT"), &cqnt)
        .expect_err("CQNT without default rejected");

    assert_invalid_payload_contains(err, "IVFP CQNT wrapper missing 'default' entry");
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

fn read_hnsw_sections(provider: &dyn IndexProvider, sections: &[Vec<u8>]) {
    for (sub_tag, bytes) in [SubTag(*b"GRPH"), SubTag(*b"VECS"), SubTag(*b"QUNT")]
        .into_iter()
        .zip(sections)
    {
        provider
            .read_section(sub_tag, bytes)
            .expect("HNSW section reads");
    }
}

fn read_ivf_sections(provider: &dyn IndexProvider, sections: &[Vec<u8>]) {
    for (sub_tag, bytes) in [SubTag(*b"CQNT"), SubTag(*b"IPQB"), SubTag(*b"POST")]
        .into_iter()
        .zip(sections)
    {
        provider
            .read_section(sub_tag, bytes)
            .expect("IVF section reads");
    }
}

fn assert_v1_wrapped(sections: &[Vec<u8>]) {
    for section in sections {
        assert!(section.starts_with(&[1, 0]));
    }
}

fn hnsw_wrapper(sub_tag: SubTag, names: &[&str]) -> Vec<u8> {
    let entries = names
        .iter()
        .map(|name| {
            let config = hnsw_config();
            let provider = HnswProvider::new(config.clone()).expect("HNSW provider builds");
            let section = provider
                .write_section(sub_tag)
                .expect("HNSW section writes");
            (
                (*name).to_owned(),
                encode_hnsw_config(&config).expect("HNSW config encodes"),
                section,
            )
        })
        .collect();
    encode_wrapper(entries)
}

fn ivf_wrapper(sub_tag: SubTag, names: &[&str]) -> Vec<u8> {
    let entries = names
        .iter()
        .map(|name| {
            let config = ivf_config();
            let provider = IvfProvider::new(config).expect("IVF provider builds");
            let section = provider.write_section(sub_tag).expect("IVF section writes");
            (
                (*name).to_owned(),
                encode_ivf_config(&config).expect("IVF config encodes"),
                section,
            )
        })
        .collect();
    encode_wrapper(entries)
}

fn encode_wrapper(entries: Vec<(String, Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (name, config, section) in entries {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(config.len() as u32).to_le_bytes());
        out.extend_from_slice(&config);
        out.extend_from_slice(&(section.len() as u32).to_le_bytes());
        out.extend_from_slice(&section);
    }
    out
}

fn assert_invalid_payload_contains(err: ProviderError, expected: &str) {
    let ProviderError::InvalidPayload { reason } = err else {
        panic!("expected invalid payload, got {err:?}");
    };
    assert!(
        reason.contains(expected),
        "expected reason to contain {expected:?}, got {reason:?}"
    );
}
