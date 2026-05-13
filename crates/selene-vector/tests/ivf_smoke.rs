//! Integration smoke tests for BRIEF-67 IVF-PQ.

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswProvider, IvfConfig, IvfProvider, IvfStats, PAYLOAD_MAGIC_IVF,
    PqParams, VectorIvfUpsertV1, VectorOp, VectorUpsertPayloadV1,
};

fn config(training_min_vectors: usize) -> IvfConfig {
    IvfConfig::with_params(
        2,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
        },
        training_min_vectors,
    )
    .unwrap()
}

fn provider(training_min_vectors: usize) -> IvfProvider {
    IvfProvider::new(config(training_min_vectors)).unwrap()
}

fn ivf_change(node_id: u64, vector: [f32; 2]) -> Change {
    let payload = VectorIvfUpsertV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(node_id),
        vector: vector.to_vec(),
    }
    .encode()
    .unwrap();
    Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").unwrap(),
        payload: Arc::from(payload),
    }
}

fn hnsw_change(node_id: u64, vector: [f32; 2]) -> Change {
    let payload = VectorUpsertPayloadV1 {
        op: VectorOp::Insert,
        node_id: NodeId::new(node_id),
        vector: vector.to_vec(),
        max_layer: 0,
    }
    .encode()
    .unwrap();
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").unwrap(),
        payload: Arc::from(payload),
    }
}

fn populate(provider: &IvfProvider, count: usize) {
    for idx in 0..count {
        provider
            .on_change(&ivf_change(
                (idx + 1) as u64,
                [idx as f32, (idx % 11) as f32],
            ))
            .unwrap();
    }
}

#[test]
fn ivf_payload_magic_vivf_pinned() {
    assert_eq!(PAYLOAD_MAGIC_IVF, *b"VIVF");
}

#[test]
fn deferred_post_persists_unassigned_rows_and_recovery_rehydrates() {
    let source = provider(256);
    populate(&source, 10);

    let cqnt = source.write_section(SubTag(*b"CQNT")).unwrap();
    let ipqb = source.write_section(SubTag(*b"IPQB")).unwrap();
    let post = source.write_section(SubTag(*b"POST")).unwrap();

    let recovered = provider(256);
    recovered.read_section(SubTag(*b"CQNT"), &cqnt).unwrap();
    recovered.read_section(SubTag(*b"IPQB"), &ipqb).unwrap();
    recovered.read_section(SubTag(*b"POST"), &post).unwrap();

    assert_eq!(recovered.snapshot().len(), 10);
    assert!(matches!(
        recovered.ivf_stats().unwrap(),
        Some(IvfStats::Deferred {
            observed_vectors: 10,
            required: 256
        })
    ));
}

#[test]
fn concurrent_vivf_between_section_writes_preserved_at_terminal_publish() {
    let provider = provider(256);
    populate(&provider, 256);

    let _cqnt = provider.write_section(SubTag(*b"CQNT")).unwrap();
    for idx in 256..261 {
        provider
            .on_change(&ivf_change(
                (idx + 1) as u64,
                [idx as f32, (idx % 11) as f32],
            ))
            .unwrap();
    }
    let _ipqb = provider.write_section(SubTag(*b"IPQB")).unwrap();
    let _post = provider.write_section(SubTag(*b"POST")).unwrap();

    match provider.ivf_stats().unwrap().unwrap() {
        IvfStats::Trained {
            posting_list_lengths,
            ..
        } => assert_eq!(posting_list_lengths.iter().sum::<u32>(), 261),
        other => panic!("expected trained stats, observed {other:?}"),
    }
}

#[test]
fn on_change_rejects_events_until_all_recovery_sections_load() {
    let source = provider(256);
    populate(&source, 10);
    let cqnt = source.write_section(SubTag(*b"CQNT")).unwrap();
    let ipqb = source.write_section(SubTag(*b"IPQB")).unwrap();

    let recovered = provider(256);
    recovered.read_section(SubTag(*b"CQNT"), &cqnt).unwrap();
    recovered.read_section(SubTag(*b"IPQB"), &ipqb).unwrap();

    let err = recovered
        .on_change(&ivf_change(50, [1.0, 1.0]))
        .unwrap_err();

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
}

#[test]
fn trained_cqnt_metric_must_match_provider_config() {
    let source = provider(256);
    populate(&source, 256);
    let cqnt = source.write_section(SubTag(*b"CQNT")).unwrap();
    let ipqb = source.write_section(SubTag(*b"IPQB")).unwrap();
    let post = source.write_section(SubTag(*b"POST")).unwrap();

    let recovered = IvfProvider::new(
        IvfConfig::with_params(
            2,
            4,
            2,
            DistanceMetric::Dot,
            PqParams {
                m_subspaces: 1,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq: false,
            },
            256,
        )
        .unwrap(),
    )
    .unwrap();
    recovered.read_section(SubTag(*b"CQNT"), &cqnt).unwrap();
    recovered.read_section(SubTag(*b"IPQB"), &ipqb).unwrap();

    let err = recovered.read_section(SubTag(*b"POST"), &post).unwrap_err();

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
}

#[test]
fn ivf_search_returns_rows_after_training() {
    let provider = provider(256);
    populate(&provider, 256);
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();

    let rows = provider
        .search(&[0.0, 0.0], 5, Some(2), None)
        .expect("search succeeds");

    assert!(!rows.is_empty());
    assert!(rows.len() <= 5);
}

#[test]
fn hnsw_ignores_vivf_events_and_ivf_ignores_vecu_events() {
    let hnsw =
        HnswProvider::new(HnswConfig::with_params(2, 16, 200, 50, DistanceMetric::L2).unwrap())
            .unwrap();
    let ivf = provider(256);

    hnsw.on_change(&ivf_change(1, [1.0, 0.0])).unwrap();
    ivf.on_change(&hnsw_change(1, [1.0, 0.0])).unwrap();

    assert_eq!(hnsw.snapshot().len(), 0);
    assert_eq!(ivf.snapshot().len(), 0);
}
