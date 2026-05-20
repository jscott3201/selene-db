#![allow(missing_docs)]
//! Criterion evidence bench for BRIEF-70 HNSW + PQ composition replay.

use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::{Change, NodeId, intern};
use selene_graph::{IndexProvider, SubTag};
use selene_vector::{
    BulkInsertRow, DistanceMetric, HnswConfig, HnswProvider, PqParams, QuantMethod,
    QuantizationConfig, VectorBulkInsertPayloadV1,
};

fn bench_composition_replay(c: &mut Criterion) {
    let corpus = corpus(320, 8);
    let mut group = c.benchmark_group("composition_replay");
    group.measurement_time(Duration::from_secs(1));
    for mode in [Mode::PlainPq, Mode::OpqPolysemous] {
        group.bench_function(
            BenchmarkId::new(mode.name(), "insert_snapshot_query"),
            |b| {
                b.iter(|| {
                    let provider = provider_for(&corpus, mode);
                    let sections = snapshot_sections(&provider);
                    let recovered = recover_provider(&sections, mode);
                    let rows = recovered
                        .search(&corpus[0], 10, Some(100), None, None)
                        .expect("composition replay search succeeds");
                    std::hint::black_box(rows);
                });
            },
        );
        group.bench_function(BenchmarkId::new(mode.name(), "insert_publish_query"), |b| {
            b.iter(|| {
                let provider = provider_for(&corpus, mode);
                let _sections = snapshot_sections(&provider);
                let rows = provider
                    .search(&corpus[0], 10, Some(100), None, None)
                    .expect("composition direct search succeeds");
                std::hint::black_box(rows);
            });
        });
    }
    group.finish();
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    PlainPq,
    OpqPolysemous,
}

impl Mode {
    const fn name(self) -> &'static str {
        match self {
            Self::PlainPq => "plain_pq",
            Self::OpqPolysemous => "opq_polysemous",
        }
    }
}

struct Sections {
    grph: Vec<u8>,
    vecs: Vec<u8>,
    qunt: Vec<u8>,
}

fn provider_for(corpus: &[Vec<f32>], mode: Mode) -> HnswProvider {
    let provider = HnswProvider::new(config_for(mode)).expect("provider config is valid");
    provider
        .on_change(&bulk_change(corpus))
        .expect("bulk insert applies");
    provider
}

fn config_for(mode: Mode) -> HnswConfig {
    let (m_subspaces, use_opq, use_polysemous) = match mode {
        Mode::PlainPq => (1, false, false),
        Mode::OpqPolysemous => (2, true, true),
    };
    HnswConfig::with_params(8, 8, 64, 50, DistanceMetric::L2)
        .expect("base config is valid")
        .with_quantization(QuantizationConfig {
            enabled: true,
            method: QuantMethod::Pq,
            rescore: false,
            pq: Some(PqParams {
                m_subspaces,
                k_centroids: 256,
                train_min_vectors: 256,
                use_opq,
                use_polysemous,
                hamming_threshold_ratio: 0.5,
            }),
        })
        .expect("PQ config is valid")
}

fn snapshot_sections(provider: &HnswProvider) -> Sections {
    let grph = provider.write_section(SubTag(*b"GRPH")).unwrap();
    let vecs = provider.write_section(SubTag(*b"VECS")).unwrap();
    let qunt = provider.write_section(SubTag(*b"QUNT")).unwrap();
    Sections { grph, vecs, qunt }
}

fn recover_provider(sections: &Sections, mode: Mode) -> HnswProvider {
    let provider = HnswProvider::new(config_for(mode)).expect("provider config is valid");
    provider
        .read_section(SubTag(*b"GRPH"), &sections.grph)
        .unwrap();
    provider
        .read_section(SubTag(*b"VECS"), &sections.vecs)
        .unwrap();
    provider
        .read_section(SubTag(*b"QUNT"), &sections.qunt)
        .unwrap();
    provider
}

fn bulk_change(corpus: &[Vec<f32>]) -> Change {
    let rows = corpus
        .iter()
        .enumerate()
        .map(|(idx, vector)| BulkInsertRow {
            node_id: NodeId::new((idx + 1) as u64),
            vector: vector.clone(),
            max_layer: 0,
        })
        .collect();
    let payload = VectorBulkInsertPayloadV1 { rows };
    Change::IndexExtensionEvent {
        provider: intern("selene-vector").expect("provider name interns"),
        payload: Arc::from(
            payload
                .encode()
                .expect("bulk payload encodes")
                .into_boxed_slice(),
        ),
    }
}

fn corpus(count: usize, dim: usize) -> Vec<Vec<f32>> {
    let mut rng = fastrand::Rng::with_seed(0xB70E_1000);
    (0..count)
        .map(|_| (0..dim).map(|_| rng.f32() * 2.0 - 1.0).collect())
        .collect()
}

criterion_group!(benches, bench_composition_replay);
criterion_main!(benches);
