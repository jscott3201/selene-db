#![allow(missing_docs)]
//! Criterion benches for standalone product-quantized vector candidate scans.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod vector_pq_support;

use std::mem::size_of;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};
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

#[derive(Clone, Copy, Debug)]
struct ScalarQuantVariant {
    name: &'static str,
    candidates: usize,
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

#[derive(Clone, Copy, Debug)]
struct BinaryQuantVariant {
    name: &'static str,
    candidates: usize,
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
struct BinaryQuantIndex {
    variant: BinaryQuantVariant,
    words_per_vector: usize,
    codes: Vec<u64>,
}

impl BinaryQuantIndex {
    fn build(vectors: &[VectorValue], variant: BinaryQuantVariant) -> Self {
        let words_per_vector = DIMENSION.div_ceil(u64::BITS as usize);
        let mut codes = vec![0; vectors.len() * words_per_vector];
        for (row, vector) in vectors.iter().enumerate() {
            encode_binary_vector(
                vector,
                &mut codes[row * words_per_vector..(row + 1) * words_per_vector],
            );
        }
        Self {
            variant,
            words_per_vector,
            codes,
        }
    }

    fn search_all(&self, vectors: &[VectorValue], query: &VectorValue, k: usize) -> Vec<usize> {
        let query_code = binary_code(query, self.words_per_vector);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in 0..vectors.len() {
            candidates.push_distance(row, f64::from(self.hamming_distance(row, &query_code)));
        }
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::SquaredEuclidean,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("binary-quant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn estimated_bytes(&self) -> usize {
        self.codes.len().saturating_mul(size_of::<u64>())
    }

    fn hamming_distance(&self, row: usize, query_code: &[u64]) -> u32 {
        let offset = row * self.words_per_vector;
        self.codes[offset..offset + self.words_per_vector]
            .iter()
            .zip(query_code)
            .map(|(left, right)| (left ^ right).count_ones())
            .sum()
    }
}

fn binary_code(vector: &VectorValue, words_per_vector: usize) -> Vec<u64> {
    let mut words = vec![0; words_per_vector];
    encode_binary_vector(vector, &mut words);
    words
}

fn encode_binary_vector(vector: &VectorValue, words: &mut [u64]) {
    words.fill(0);
    for (dim, value) in vector.as_slice().iter().enumerate() {
        if *value >= 0.0 {
            words[dim / u64::BITS as usize] |= 1_u64 << (dim % u64::BITS as usize);
        }
    }
}

#[derive(Debug)]
struct ScalarQuantIndex {
    variant: ScalarQuantVariant,
    mins: Vec<f32>,
    scales: Vec<f32>,
    codes: Vec<u8>,
}

impl ScalarQuantIndex {
    fn build(vectors: &[VectorValue], variant: ScalarQuantVariant) -> Self {
        let (mins, scales) = scalar_ranges(vectors);
        let codes = encode_scalar_vectors(vectors, &mins, &scales);
        Self {
            variant,
            mins,
            scales,
            codes,
        }
    }

    fn search_all(&self, vectors: &[VectorValue], query: &VectorValue, k: usize) -> Vec<usize> {
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in 0..vectors.len() {
            candidates.push_distance(row, self.approx_distance(row, query));
        }
        let candidate_ids = candidates
            .into_hits()
            .into_iter()
            .map(|hit| hit.key)
            .collect::<Vec<_>>();
        let hits = exact_vector_top_k(
            VectorMetric::SquaredEuclidean,
            query,
            candidate_ids.iter().map(|&row| (row, &vectors[row])),
            k,
        )
        .expect("scalar-quant benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn estimated_bytes(&self) -> usize {
        self.codes.len().saturating_add(
            self.mins
                .len()
                .saturating_add(self.scales.len())
                .saturating_mul(size_of::<f32>()),
        )
    }

    fn approx_distance(&self, row: usize, query: &VectorValue) -> f64 {
        let code_offset = row * DIMENSION;
        query
            .as_slice()
            .iter()
            .enumerate()
            .map(|(dim, query_value)| {
                let code = f32::from(self.codes[code_offset + dim]);
                let decoded = self.mins[dim] + self.scales[dim] * code;
                let delta = f64::from(*query_value) - f64::from(decoded);
                delta * delta
            })
            .sum()
    }
}

fn scalar_ranges(vectors: &[VectorValue]) -> (Vec<f32>, Vec<f32>) {
    let mut mins = vec![f32::INFINITY; DIMENSION];
    let mut maxs = vec![f32::NEG_INFINITY; DIMENSION];
    for vector in vectors {
        for (dim, value) in vector.as_slice().iter().enumerate() {
            mins[dim] = mins[dim].min(*value);
            maxs[dim] = maxs[dim].max(*value);
        }
    }
    let scales = mins
        .iter()
        .zip(maxs)
        .map(|(min, max)| {
            let span = max - *min;
            if span > 0.0 { span / 255.0 } else { 1.0 }
        })
        .collect();
    (mins, scales)
}

fn encode_scalar_vectors(vectors: &[VectorValue], mins: &[f32], scales: &[f32]) -> Vec<u8> {
    let mut codes = vec![0; vectors.len() * DIMENSION];
    for (row, vector) in vectors.iter().enumerate() {
        for (dim, value) in vector.as_slice().iter().enumerate() {
            let scaled = ((*value - mins[dim]) / scales[dim]).round();
            codes[row * DIMENSION + dim] = scaled.clamp(0.0, 255.0) as u8;
        }
    }
    codes
}

criterion_group! {
    name = vector_pq;
    config = common::criterion_config();
    targets =
        bench_pq_candidate_recall,
        bench_scalar_quant_candidate_recall,
        bench_binary_quant_candidate_recall
}
criterion_main!(vector_pq);
