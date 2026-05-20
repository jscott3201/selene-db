//! IVF numeric-correctness regression tests for BRIEF-95a.

use std::sync::Arc;

use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::{
    DistanceMetric, IvfConfig, IvfProvider, PqParams, VectorIvfUpsertV1, VectorOp,
};

fn dot_config(use_polysemous: bool) -> IvfConfig {
    IvfConfig::with_params(
        4,
        4,
        4,
        DistanceMetric::Dot,
        PqParams {
            m_subspaces: 2,
            k_centroids: 256,
            train_min_vectors: 256,
            use_opq: false,
            use_polysemous,
            hamming_threshold_ratio: 0.0,
        },
        256,
    )
    .unwrap()
}

fn ivf_change(node_id: u64, vector: &[f32]) -> Change {
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

fn corpus() -> Vec<Vec<f32>> {
    let mut rng = fastrand::Rng::with_seed(0xB95A_1F00);
    (0..320)
        .map(|idx| {
            let phase = idx as f32 * 0.037;
            vec![
                (rng.f32() * 2.0 - 1.0) + phase.sin() * 0.3,
                (rng.f32() * 2.0 - 1.0) + phase.cos() * 0.2,
                (rng.f32() * 2.0 - 1.0) - phase.sin() * 0.1,
                (rng.f32() * 2.0 - 1.0) + phase.cos() * 0.4,
            ]
        })
        .collect()
}

fn populate(provider: &IvfProvider, rows: &[Vec<f32>]) {
    for (idx, row) in rows.iter().enumerate() {
        provider
            .on_change(&ivf_change((idx + 1) as u64, row))
            .unwrap();
    }
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();
}

fn result_ids(results: &[(NodeId, f32)]) -> Vec<NodeId> {
    results.iter().map(|(node_id, _)| *node_id).collect()
}

fn exact_dot_top_k(rows: &[Vec<f32>], query: &[f32], k: usize) -> Vec<NodeId> {
    let mut scored = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let dot = row.iter().zip(query).map(|(a, b)| a * b).sum::<f32>();
            (NodeId::new((idx + 1) as u64), dot)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored
        .into_iter()
        .take(k)
        .map(|(node_id, _)| node_id)
        .collect()
}

fn recall_at_10(results: &[NodeId], exact: &[NodeId]) -> usize {
    results
        .iter()
        .filter(|node_id| exact.contains(node_id))
        .count()
}

#[test]
fn dot_metric_polysemous_search_matches_no_polysemous_baseline() {
    let rows = corpus();
    let query = [0.4, -0.7, 0.2, 0.9];
    let plain = IvfProvider::new(dot_config(false)).unwrap();
    let polysemous = IvfProvider::new(dot_config(true)).unwrap();
    populate(&plain, &rows);
    populate(&polysemous, &rows);

    let plain_ids = result_ids(&plain.search(&query, 10, Some(4), None, None).unwrap());
    let polysemous_ids = result_ids(&polysemous.search(&query, 10, Some(4), None, None).unwrap());
    let exact = exact_dot_top_k(&rows, &query, 10);

    assert_eq!(polysemous_ids, plain_ids);
    assert!(recall_at_10(&polysemous_ids, &exact) >= recall_at_10(&plain_ids, &exact));
}
