#![allow(missing_docs)]
//! Criterion benches for standalone product-quantized vector candidate scans.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod vector_pq_support;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vector_pq_support::{DIMENSION, K, PqCorpus, PqIndex, PqVariant, memory_suffix, vector_scales};

const VARIANTS: [PqVariant; 6] = [
    PqVariant {
        name: "m16_k16_c64",
        subvectors: 16,
        codewords: 16,
        candidates: 64,
    },
    PqVariant {
        name: "m16_k16_c256",
        subvectors: 16,
        codewords: 16,
        candidates: 256,
    },
    PqVariant {
        name: "m16_k16_c1024",
        subvectors: 16,
        codewords: 16,
        candidates: 1024,
    },
    PqVariant {
        name: "m16_k64_c64",
        subvectors: 16,
        codewords: 64,
        candidates: 64,
    },
    PqVariant {
        name: "m16_k64_c256",
        subvectors: 16,
        codewords: 64,
        candidates: 256,
    },
    PqVariant {
        name: "m16_k64_c1024",
        subvectors: 16,
        codewords: 64,
        candidates: 1024,
    },
];

fn bench_pq_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_pq_candidate_recall");
    for scale in vector_scales() {
        for variant in VARIANTS {
            let fixture = PqFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                (fixture.corpus.scale * fixture.corpus.queries.len()) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_l2",
                    format!(
                        "{}_d{DIMENSION}_k{K}_recallbp{}_{}",
                        variant.name,
                        fixture.recall_basis_points(),
                        fixture.memory_suffix()
                    ),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(fixture.total_overlap());
                    });
                },
            );
        }
    }
    group.finish();
}

#[derive(Debug)]
struct PqFixture {
    corpus: PqCorpus,
    index: PqIndex,
}

impl PqFixture {
    fn build(scale: usize, variant: PqVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let index = PqIndex::train(&corpus.vectors, variant);
        Self { corpus, index }
    }

    fn total_overlap(&self) -> usize {
        self.corpus
            .total_overlap(|query| self.index.search_all(&self.corpus.vectors, query, K))
    }

    fn recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.total_overlap())
    }

    fn memory_suffix(&self) -> String {
        memory_suffix(
            self.index.estimated_bytes(),
            self.corpus.full_vector_bytes(),
        )
    }
}

criterion_group! {
    name = vector_pq;
    config = common::criterion_config();
    targets = bench_pq_candidate_recall
}
criterion_main!(vector_pq);
