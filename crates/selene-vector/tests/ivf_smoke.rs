//! Integration smoke tests for BRIEF-67 IVF-PQ.

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, ProviderError, SubTag};
use selene_vector::{
    DistanceMetric, HnswConfig, HnswProvider, IvfBulkInsertRow, IvfConfig, IvfProvider, IvfStats,
    PAYLOAD_MAGIC_IVF, PAYLOAD_MAGIC_IVF_BULK_DELETE, PAYLOAD_MAGIC_IVF_BULK_INSERT, PqParams,
    VectorIvfBulkDeleteV1, VectorIvfBulkInsertV1, VectorIvfUpsertV1, VectorOp,
    VectorUpsertPayloadV1,
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
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
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

fn ivf_delete_change(node_id: u64) -> Change {
    let payload = VectorIvfUpsertV1 {
        op: VectorOp::Delete,
        node_id: NodeId::new(node_id),
        vector: Vec::new(),
    }
    .encode()
    .unwrap();
    Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").unwrap(),
        payload: Arc::from(payload),
    }
}

fn ivf_bulk_insert_change(rows: &[(u64, [f32; 2])]) -> Change {
    let payload = VectorIvfBulkInsertV1 {
        rows: rows
            .iter()
            .map(|(node_id, vector)| IvfBulkInsertRow {
                node_id: NodeId::new(*node_id),
                vector: vector.to_vec(),
            })
            .collect(),
    }
    .encode()
    .unwrap();
    Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").unwrap(),
        payload: Arc::from(payload),
    }
}

fn ivf_bulk_delete_change(node_ids: &[u64]) -> Change {
    let payload = VectorIvfBulkDeleteV1 {
        node_ids: node_ids
            .iter()
            .map(|node_id| NodeId::new(*node_id))
            .collect(),
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
    assert_eq!(PAYLOAD_MAGIC_IVF_BULK_INSERT, *b"VIVB");
    assert_eq!(PAYLOAD_MAGIC_IVF_BULK_DELETE, *b"VIVD");
}

#[test]
fn ivf_stats_trained_reports_polysemous_flag() {
    // §H #36 / V116: `IvfStats::Trained` surfaces `polysemous` from the
    // residual codebook so observers (the diagnostic procedure, packs,
    // ops dashboards) can verify the filter is actually in effect.
    // Polysemous requires m_subspaces >= 2 and a residual distribution
    // that k-means can fit; use dim=4 with synthetic random vectors so
    // each subspace has 2 dims and 256 distinct residuals per subspace.
    fn polysemous_ivf_change(node_id: u64, vector: [f32; 4]) -> Change {
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
    fn populate_random(provider: &IvfProvider, count: usize, seed: u64) {
        let mut rng = fastrand::Rng::with_seed(seed);
        for idx in 0..count {
            let vec = [
                (rng.f32() * 2.0) - 1.0,
                (rng.f32() * 2.0) - 1.0,
                (rng.f32() * 2.0) - 1.0,
                (rng.f32() * 2.0) - 1.0,
            ];
            provider
                .on_change(&polysemous_ivf_change((idx + 1) as u64, vec))
                .unwrap();
        }
    }
    let poly_config = IvfConfig::with_params(
        4,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: true,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .unwrap();
    let poly = IvfProvider::new(poly_config).unwrap();
    populate_random(&poly, 256, 0xB69E_6001);
    poly.write_section(SubTag(*b"CQNT")).unwrap();
    poly.write_section(SubTag(*b"IPQB")).unwrap();
    poly.write_section(SubTag(*b"POST")).unwrap();
    match poly.ivf_stats().unwrap().unwrap() {
        IvfStats::Trained {
            polysemous: true, ..
        } => {}
        other => panic!("expected polysemous=true, observed {other:?}"),
    }

    let plain_config = IvfConfig::with_params(
        4,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        256,
    )
    .unwrap();
    let plain = IvfProvider::new(plain_config).unwrap();
    populate_random(&plain, 256, 0xB69E_6001);
    plain.write_section(SubTag(*b"CQNT")).unwrap();
    plain.write_section(SubTag(*b"IPQB")).unwrap();
    plain.write_section(SubTag(*b"POST")).unwrap();
    match plain.ivf_stats().unwrap().unwrap() {
        IvfStats::Trained {
            polysemous: false, ..
        } => {}
        other => panic!("expected polysemous=false, observed {other:?}"),
    }
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
                use_polysemous: false,
                hamming_threshold_ratio: 0.5,
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
fn ivf_delete_removes_node_from_search_candidates() {
    let provider = provider(256);
    populate(&provider, 256);
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();

    provider
        .on_change(&ivf_delete_change(3))
        .expect("delete succeeds");
    let rows = provider
        .search(&[2.0, 2.0], 256, Some(4), None)
        .expect("search succeeds");

    assert!(!rows.iter().any(|(node_id, _)| *node_id == NodeId::new(3)));
    assert_eq!(provider.snapshot().len(), 255);
}

#[test]
fn ivf_delete_untrained_index_is_idempotent() {
    let provider = provider(256);

    provider
        .on_change(&ivf_delete_change(404))
        .expect("missing delete succeeds");

    assert!(provider.snapshot().is_empty());
}

#[test]
fn ivf_delete_existing_deferred_row_not_resurrected_on_train() {
    let provider = provider(256);
    populate(&provider, 255);
    provider
        .on_change(&ivf_delete_change(2))
        .expect("deferred delete succeeds");
    provider
        .on_change(&ivf_change(256, [256.0, 3.0]))
        .expect("insert succeeds");
    provider
        .on_change(&ivf_change(257, [257.0, 4.0]))
        .expect("insert succeeds");

    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();

    let snapshot = provider.snapshot();
    assert!(snapshot.is_trained());
    assert_eq!(snapshot.len(), 256);
    let rows = provider
        .search(&[1.0, 1.0], 300, Some(4), None)
        .expect("search succeeds");
    assert!(!rows.iter().any(|(node_id, _)| *node_id == NodeId::new(2)));
}

#[test]
fn ivf_bulk_insert_untrained_appends_all_rows_to_deferred_buffer() {
    let provider = provider(256);

    provider
        .on_change(&ivf_bulk_insert_change(&[
            (1, [1.0, 0.0]),
            (2, [2.0, 0.0]),
            (3, [3.0, 0.0]),
        ]))
        .expect("bulk insert succeeds");

    let snapshot = provider.snapshot();
    assert!(!snapshot.is_trained());
    assert_eq!(snapshot.len(), 3);
    assert!(matches!(
        provider.ivf_stats().unwrap(),
        Some(IvfStats::Deferred {
            observed_vectors: 3,
            required: 256
        })
    ));
}

#[test]
fn ivf_bulk_insert_trained_routes_rows_to_postings_without_retraining() {
    let provider = provider(256);
    populate(&provider, 256);
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();

    provider
        .on_change(&ivf_bulk_insert_change(&[
            (300, [300.0, 3.0]),
            (301, [301.0, 4.0]),
        ]))
        .expect("bulk insert succeeds");

    match provider.ivf_stats().unwrap().unwrap() {
        IvfStats::Trained {
            posting_list_lengths,
            unassigned_count,
            ..
        } => {
            assert_eq!(posting_list_lengths.iter().sum::<u32>(), 258);
            assert_eq!(unassigned_count, 0);
        }
        other => panic!("expected trained stats, observed {other:?}"),
    }
}

#[test]
fn ivf_bulk_insert_rejects_existing_duplicate_atomically() {
    let provider = provider(256);
    populate(&provider, 1);

    let err = provider
        .on_change(&ivf_bulk_insert_change(&[(1, [9.0, 9.0]), (2, [2.0, 0.0])]))
        .expect_err("existing duplicate rejected");

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
    assert_eq!(provider.snapshot().len(), 1);
}

#[test]
fn ivf_bulk_insert_rejects_bad_row_before_partial_publish() {
    let provider = provider(256);
    let payload = VectorIvfBulkInsertV1 {
        rows: vec![
            IvfBulkInsertRow {
                node_id: NodeId::new(1),
                vector: vec![1.0, 0.0],
            },
            IvfBulkInsertRow {
                node_id: NodeId::new(2),
                vector: vec![f32::NAN, 0.0],
            },
        ],
    }
    .encode()
    .unwrap();
    let change = Change::IndexExtensionEvent {
        provider: intern("selene-vector-ivf").unwrap(),
        payload: Arc::from(payload),
    };

    let err = provider
        .on_change(&change)
        .expect_err("non-finite row rejected");

    assert!(matches!(err, ProviderError::InvalidPayload { .. }));
    assert!(provider.snapshot().is_empty());
}

#[test]
fn ivf_bulk_delete_removes_deferred_rows_and_is_idempotent() {
    let provider = provider(256);
    populate(&provider, 5);

    provider
        .on_change(&ivf_bulk_delete_change(&[2, 4, 99]))
        .expect("bulk delete succeeds");

    let snapshot = provider.snapshot();
    assert_eq!(snapshot.len(), 3);
    assert!(matches!(
        provider.ivf_stats().unwrap(),
        Some(IvfStats::Deferred {
            observed_vectors: 3,
            required: 256
        })
    ));
}

#[test]
fn ivf_bulk_delete_removes_trained_posting_entries_from_search() {
    let provider = provider(256);
    populate(&provider, 256);
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();

    provider
        .on_change(&ivf_bulk_delete_change(&[3, 4, 5]))
        .expect("bulk delete succeeds");

    let rows = provider
        .search(&[2.0, 2.0], 256, Some(4), None)
        .expect("search succeeds");
    assert_eq!(provider.snapshot().len(), 253);
    for node_id in [3, 4, 5] {
        assert!(
            !rows
                .iter()
                .any(|(candidate, _)| *candidate == NodeId::new(node_id))
        );
    }
}

#[test]
fn ivf_out_of_order_writes_clear_staging_and_recover() {
    // BRIEF-69 §I-10 / Local Codex F2 behavioral coverage. After an
    // out-of-order section write (IPQB before CQNT, POST before
    // CQNT+IPQB), staging must be back at Idle so a fresh CQNT→IPQB→POST
    // sequence completes cleanly. A lingering `Writing` would corrupt the
    // next snapshot attempt by feeding it stale `captured`.
    let provider = provider(256);
    populate(&provider, 256);

    let err = provider.write_section(SubTag(*b"IPQB")).unwrap_err();
    assert!(
        matches!(err, ProviderError::InvalidPayload { .. }),
        "IPQB before CQNT must error: {err:?}"
    );
    let err = provider.write_section(SubTag(*b"POST")).unwrap_err();
    assert!(
        matches!(err, ProviderError::InvalidPayload { .. }),
        "POST before CQNT+IPQB must error: {err:?}"
    );

    // Clean sequence must now succeed; if staging had leaked to `Writing`
    // the second CQNT call would either re-emit stale `captured` or POST
    // would publish an inconsistent merge.
    let _cqnt = provider.write_section(SubTag(*b"CQNT")).unwrap();
    let _ipqb = provider.write_section(SubTag(*b"IPQB")).unwrap();
    let _post = provider.write_section(SubTag(*b"POST")).unwrap();
    match provider.ivf_stats().unwrap().unwrap() {
        IvfStats::Trained {
            posting_list_lengths,
            ..
        } => assert_eq!(posting_list_lengths.iter().sum::<u32>(), 256),
        other => panic!("expected trained stats after recovery, observed {other:?}"),
    }
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
