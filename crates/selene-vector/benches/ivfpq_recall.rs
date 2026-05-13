#![allow(missing_docs)]
//! Criterion smoke benchmark for IVF-PQ search.

use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::{
    DistanceMetric, IvfConfig, IvfProvider, PqParams, VectorIvfUpsertV1, VectorOp,
};

fn bench_ivfpq_recall(c: &mut Criterion) {
    let corpus = synthetic_corpus(256, 8);
    let mut group = c.benchmark_group("vector_ivfpq_recall_at_10");
    group.measurement_time(Duration::from_secs(1));
    for n_probe in [1_u32, 4, 8] {
        let provider = provider_for(&corpus, n_probe);
        group.bench_function(BenchmarkId::new("n_probe", n_probe), |b| {
            b.iter(|| {
                let rows = provider
                    .search(&corpus[0], 10, Some(n_probe), None)
                    .expect("IVF search succeeds");
                std::hint::black_box(rows);
            });
        });
    }
    group.finish();
}

fn provider_for(corpus: &[Vec<f32>], n_probe: u32) -> IvfProvider {
    let config = IvfConfig::with_params(
        corpus[0].len(),
        16,
        n_probe,
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
    .expect("bench config valid");
    let provider = IvfProvider::new(config).expect("provider config valid");
    for (idx, vector) in corpus.iter().enumerate() {
        let payload = VectorIvfUpsertV1 {
            op: VectorOp::Insert,
            node_id: NodeId::new((idx + 1) as u64),
            vector: vector.clone(),
        }
        .encode()
        .expect("payload encodes");
        provider
            .on_change(&Change::IndexExtensionEvent {
                provider: intern("selene-vector-ivf").unwrap(),
                payload: Arc::from(payload),
            })
            .expect("insert applies");
    }
    provider.write_section(SubTag(*b"CQNT")).unwrap();
    provider.write_section(SubTag(*b"IPQB")).unwrap();
    provider.write_section(SubTag(*b"POST")).unwrap();
    provider
}

fn synthetic_corpus(count: usize, dim: usize) -> Vec<Vec<f32>> {
    let mut rng = fastrand::Rng::with_seed(0xB67E_0010);
    (0..count)
        .map(|_| (0..dim).map(|_| rng.f32() * 2.0 - 1.0).collect())
        .collect()
}

criterion_group!(benches, bench_ivfpq_recall);
criterion_main!(benches);
