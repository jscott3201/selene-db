#![allow(missing_docs)]
//! Criterion benches for standalone product-quantized vector candidate scans.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod vector_pq_support;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vector_pq_support::turbo_quant::{TurboQuantIndex, TurboQuantVariant};
use vector_pq_support::{
    BinaryQuantIndex, BinaryQuantVariant, DIMENSION, K, PqCorpus, PqIndex, PqVariant,
    ScalarQuantIndex, ScalarQuantVariant, memory_suffix, vector_scales,
};

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

const SCALAR_VARIANTS: [ScalarQuantVariant; 3] = [
    ScalarQuantVariant {
        name: "u8_c64",
        candidates: 64,
    },
    ScalarQuantVariant {
        name: "u8_c256",
        candidates: 256,
    },
    ScalarQuantVariant {
        name: "u8_c1024",
        candidates: 1024,
    },
];

const SCALAR_CODE_VARIANTS: [ScalarQuantVariant; 3] = [
    ScalarQuantVariant {
        name: "u8code_c64",
        candidates: 64,
    },
    ScalarQuantVariant {
        name: "u8code_c256",
        candidates: 256,
    },
    ScalarQuantVariant {
        name: "u8code_c1024",
        candidates: 1024,
    },
];

const BINARY_VARIANTS: [BinaryQuantVariant; 4] = [
    BinaryQuantVariant {
        name: "sign_c32",
        candidates: 32,
    },
    BinaryQuantVariant {
        name: "sign_c64",
        candidates: 64,
    },
    BinaryQuantVariant {
        name: "sign_c256",
        candidates: 256,
    },
    BinaryQuantVariant {
        name: "sign_c1024",
        candidates: 1024,
    },
];

const TURBO_QUANT_VARIANTS: [TurboQuantVariant; 7] = [
    TurboQuantVariant {
        name: "tq2_c256",
        bit_width: 2,
        candidates: 256,
    },
    TurboQuantVariant {
        name: "tq2_c1024",
        bit_width: 2,
        candidates: 1024,
    },
    TurboQuantVariant {
        name: "tq3_c256",
        bit_width: 3,
        candidates: 256,
    },
    TurboQuantVariant {
        name: "tq3_c1024",
        bit_width: 3,
        candidates: 1024,
    },
    TurboQuantVariant {
        name: "tq4_c64",
        bit_width: 4,
        candidates: 64,
    },
    TurboQuantVariant {
        name: "tq4_c256",
        bit_width: 4,
        candidates: 256,
    },
    TurboQuantVariant {
        name: "tq4_c1024",
        bit_width: 4,
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

fn bench_scalar_quant_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_scalar_quant_candidate_recall");
    for scale in vector_scales() {
        for variant in SCALAR_VARIANTS {
            let fixture = ScalarQuantFixture::build(scale, variant);
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

fn bench_scalar_code_quant_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_scalar_code_quant_candidate_recall");
    for scale in vector_scales() {
        for variant in SCALAR_CODE_VARIANTS {
            let fixture = ScalarQuantFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                (fixture.corpus.scale * fixture.corpus.queries.len()) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_l2",
                    format!(
                        "{}_d{DIMENSION}_k{K}_recallbp{}_{}",
                        variant.name,
                        fixture.code_recall_basis_points(),
                        fixture.memory_suffix()
                    ),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(fixture.code_total_overlap());
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_binary_quant_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_binary_quant_candidate_recall");
    for scale in vector_scales() {
        for variant in BINARY_VARIANTS {
            let fixture = BinaryQuantFixture::build(scale, variant);
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

fn bench_turbo_quant_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_turbo_quant_candidate_recall");
    for scale in vector_scales() {
        for variant in TURBO_QUANT_VARIANTS {
            let fixture = TurboQuantFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                (fixture.corpus.scale * fixture.corpus.queries.len()) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_cos",
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

#[derive(Debug)]
struct ScalarQuantFixture {
    corpus: PqCorpus,
    index: ScalarQuantIndex,
}

impl ScalarQuantFixture {
    fn build(scale: usize, variant: ScalarQuantVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let index = ScalarQuantIndex::build(&corpus.vectors, variant);
        Self { corpus, index }
    }

    fn total_overlap(&self) -> usize {
        self.corpus
            .total_overlap(|query| self.index.search_all(&self.corpus.vectors, query, K))
    }

    fn code_total_overlap(&self) -> usize {
        self.corpus.total_overlap(|query| {
            self.index
                .search_all_code_l2(&self.corpus.vectors, query, K)
        })
    }

    fn recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.total_overlap())
    }

    fn code_recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.code_total_overlap())
    }

    fn memory_suffix(&self) -> String {
        memory_suffix(
            self.index.estimated_bytes(),
            self.corpus.full_vector_bytes(),
        )
    }
}

#[derive(Debug)]
struct BinaryQuantFixture {
    corpus: PqCorpus,
    index: BinaryQuantIndex,
}

impl BinaryQuantFixture {
    fn build(scale: usize, variant: BinaryQuantVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let index = BinaryQuantIndex::build(&corpus.vectors, variant);
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

#[derive(Debug)]
struct TurboQuantFixture {
    corpus: PqCorpus,
    index: TurboQuantIndex,
}

impl TurboQuantFixture {
    fn build(scale: usize, variant: TurboQuantVariant) -> Self {
        let corpus = PqCorpus::build_cosine(scale);
        let index = TurboQuantIndex::build(&corpus.vectors, variant);
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
    targets =
        bench_pq_candidate_recall,
        bench_scalar_quant_candidate_recall,
        bench_scalar_code_quant_candidate_recall,
        bench_binary_quant_candidate_recall,
        bench_turbo_quant_candidate_recall
}
criterion_main!(vector_pq);
