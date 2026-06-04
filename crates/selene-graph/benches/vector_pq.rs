#![allow(missing_docs)]
//! Criterion benches for product-quantized vector candidate generation.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::mem::size_of;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{VectorMetric, VectorTopK, VectorValue, exact_vector_top_k};
use selene_testing::BenchProfile;

const DIMENSION: usize = 128;
const QUERY_COUNT: usize = 16;
const K: usize = 10;
const TRAINING_ITERATIONS: usize = 2;
const TRAINING_SAMPLE_MAX: usize = 8_192;

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

#[derive(Clone, Copy, Debug)]
struct PqVariant {
    name: &'static str,
    subvectors: usize,
    codewords: usize,
    candidates: usize,
}

fn bench_pq_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_pq_candidate_recall");
    for scale in vector_scales() {
        for variant in VARIANTS {
            let fixture = PqFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                (fixture.scale * fixture.queries.len()) as u64,
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

fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(parse_scales)
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn parse_scales(raw: String) -> Option<Vec<usize>> {
    let mut scales: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|scale| *scale > 0)
        .collect();
    scales.sort_unstable();
    scales.dedup();
    (!scales.is_empty()).then_some(scales)
}

#[derive(Debug)]
struct PqFixture {
    scale: usize,
    vectors: Vec<VectorValue>,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<usize>>,
    index: PqIndex,
}

impl PqFixture {
    fn build(scale: usize, variant: PqVariant) -> Self {
        let scale = scale.max(K);
        let vectors = (0..scale)
            .map(|seed| clustered_vector(seed, scale, 0.0))
            .collect::<Vec<_>>();
        let queries = (0..QUERY_COUNT)
            .map(|idx| {
                let cluster_count = cluster_count(scale);
                let cluster = idx % cluster_count;
                let seed = cluster + (scale / cluster_count / 2) * cluster_count;
                clustered_vector(seed.min(scale - 1), scale, 0.0003)
            })
            .collect::<Vec<_>>();
        let exact = queries
            .iter()
            .map(|query| exact_ids(&vectors, query, K))
            .collect::<Vec<_>>();
        let index = PqIndex::train(&vectors, variant);
        Self {
            scale,
            vectors,
            queries,
            exact,
            index,
        }
    }

    fn total_overlap(&self) -> usize {
        self.queries
            .iter()
            .zip(&self.exact)
            .map(|(query, exact)| {
                let approx = self.index.search(&self.vectors, query, K);
                exact.iter().filter(|id| approx.contains(id)).count()
            })
            .sum()
    }

    fn recall_basis_points(&self) -> usize {
        self.total_overlap() * 10_000 / (self.queries.len() * K)
    }

    fn memory_suffix(&self) -> String {
        let full_bytes = self
            .vectors
            .len()
            .saturating_mul(DIMENSION)
            .saturating_mul(size_of::<f32>());
        let compressed = self.index.estimated_bytes();
        format!("m{}-full{}", compressed / 1024, full_bytes / 1024)
    }
}

#[derive(Debug)]
struct PqIndex {
    variant: PqVariant,
    subdim: usize,
    centroids: Vec<f32>,
    codes: Vec<u8>,
}

impl PqIndex {
    fn train(vectors: &[VectorValue], variant: PqVariant) -> Self {
        assert_eq!(DIMENSION % variant.subvectors, 0);
        assert!(variant.codewords <= usize::from(u8::MAX) + 1);
        let subdim = DIMENSION / variant.subvectors;
        let sample = training_sample(vectors.len());
        let mut centroids = vec![0.0; variant.subvectors * variant.codewords * subdim];
        for subvector in 0..variant.subvectors {
            seed_centroids(vectors, &sample, variant, subdim, subvector, &mut centroids);
            refine_centroids(vectors, &sample, variant, subdim, subvector, &mut centroids);
        }
        let codes = encode_vectors(vectors, variant, subdim, &centroids);
        Self {
            variant,
            subdim,
            centroids,
            codes,
        }
    }

    fn search(&self, vectors: &[VectorValue], query: &VectorValue, k: usize) -> Vec<usize> {
        let table = self.distance_table(query);
        let mut candidates = VectorTopK::new(self.variant.candidates.max(k));
        for row in 0..vectors.len() {
            candidates.push_distance(row, self.approx_distance(row, &table));
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
        .expect("PQ benchmark vectors have matching dimensions");
        hits.into_iter().map(|hit| hit.key).collect()
    }

    fn distance_table(&self, query: &VectorValue) -> Vec<f64> {
        let mut table = vec![0.0; self.variant.subvectors * self.variant.codewords];
        for subvector in 0..self.variant.subvectors {
            let query_offset = subvector * self.subdim;
            let query_part = &query.as_slice()[query_offset..query_offset + self.subdim];
            for codeword in 0..self.variant.codewords {
                let centroid = centroid_slice(
                    &self.centroids,
                    self.variant,
                    self.subdim,
                    subvector,
                    codeword,
                );
                table[subvector * self.variant.codewords + codeword] =
                    squared_l2(query_part, centroid);
            }
        }
        table
    }

    fn approx_distance(&self, row: usize, table: &[f64]) -> f64 {
        let code_offset = row * self.variant.subvectors;
        let mut distance = 0.0;
        for subvector in 0..self.variant.subvectors {
            let codeword = usize::from(self.codes[code_offset + subvector]);
            distance += table[subvector * self.variant.codewords + codeword];
        }
        distance
    }

    fn estimated_bytes(&self) -> usize {
        self.codes
            .len()
            .saturating_add(self.centroids.len().saturating_mul(size_of::<f32>()))
    }
}

fn exact_ids(vectors: &[VectorValue], query: &VectorValue, k: usize) -> Vec<usize> {
    exact_vector_top_k(
        VectorMetric::SquaredEuclidean,
        query,
        vectors.iter().enumerate(),
        k,
    )
    .expect("PQ benchmark vectors have matching dimensions")
    .into_iter()
    .map(|hit| hit.key)
    .collect()
}

fn training_sample(len: usize) -> Vec<usize> {
    let sample_len = len.min(TRAINING_SAMPLE_MAX);
    if sample_len == 0 {
        return Vec::new();
    }
    if sample_len == 1 {
        return vec![0];
    }
    let last = len - 1;
    (0..sample_len)
        .map(|slot| slot.saturating_mul(last) / (sample_len - 1))
        .collect()
}

fn seed_centroids(
    vectors: &[VectorValue],
    sample: &[usize],
    variant: PqVariant,
    subdim: usize,
    subvector: usize,
    centroids: &mut [f32],
) {
    let last = sample.len() - 1;
    for codeword in 0..variant.codewords {
        let source = if variant.codewords == 1 {
            sample[0]
        } else {
            sample[codeword.saturating_mul(last) / (variant.codewords - 1)]
        };
        let source_part = subvector_slice(&vectors[source], subvector, subdim);
        centroid_slice_mut(centroids, variant, subdim, subvector, codeword)
            .copy_from_slice(source_part);
    }
}

fn refine_centroids(
    vectors: &[VectorValue],
    sample: &[usize],
    variant: PqVariant,
    subdim: usize,
    subvector: usize,
    centroids: &mut [f32],
) {
    for _ in 0..TRAINING_ITERATIONS {
        let mut sums = vec![0.0f64; variant.codewords * subdim];
        let mut counts = vec![0usize; variant.codewords];
        for &row in sample {
            let part = subvector_slice(&vectors[row], subvector, subdim);
            let nearest = nearest_centroid(part, centroids, variant, subdim, subvector);
            counts[nearest] += 1;
            for dim in 0..subdim {
                sums[nearest * subdim + dim] += f64::from(part[dim]);
            }
        }
        for (codeword, count) in counts.into_iter().enumerate() {
            if count == 0 {
                continue;
            }
            let inverse = 1.0 / count as f64;
            let centroid = centroid_slice_mut(centroids, variant, subdim, subvector, codeword);
            for dim in 0..subdim {
                centroid[dim] = (sums[codeword * subdim + dim] * inverse) as f32;
            }
        }
    }
}

fn encode_vectors(
    vectors: &[VectorValue],
    variant: PqVariant,
    subdim: usize,
    centroids: &[f32],
) -> Vec<u8> {
    let mut codes = vec![0; vectors.len() * variant.subvectors];
    for (row, vector) in vectors.iter().enumerate() {
        for subvector in 0..variant.subvectors {
            let part = subvector_slice(vector, subvector, subdim);
            let nearest = nearest_centroid(part, centroids, variant, subdim, subvector);
            codes[row * variant.subvectors + subvector] =
                u8::try_from(nearest).expect("PQ codeword fits u8");
        }
    }
    codes
}

fn nearest_centroid(
    part: &[f32],
    centroids: &[f32],
    variant: PqVariant,
    subdim: usize,
    subvector: usize,
) -> usize {
    let mut best = 0;
    let mut best_distance = f64::INFINITY;
    for codeword in 0..variant.codewords {
        let centroid = centroid_slice(centroids, variant, subdim, subvector, codeword);
        let distance = squared_l2(part, centroid);
        if distance
            .total_cmp(&best_distance)
            .then_with(|| codeword.cmp(&best))
            .is_lt()
        {
            best = codeword;
            best_distance = distance;
        }
    }
    best
}

fn centroid_slice(
    centroids: &[f32],
    variant: PqVariant,
    subdim: usize,
    subvector: usize,
    codeword: usize,
) -> &[f32] {
    let offset = (subvector * variant.codewords + codeword) * subdim;
    &centroids[offset..offset + subdim]
}

fn centroid_slice_mut(
    centroids: &mut [f32],
    variant: PqVariant,
    subdim: usize,
    subvector: usize,
    codeword: usize,
) -> &mut [f32] {
    let offset = (subvector * variant.codewords + codeword) * subdim;
    &mut centroids[offset..offset + subdim]
}

fn subvector_slice(vector: &VectorValue, subvector: usize, subdim: usize) -> &[f32] {
    let offset = subvector * subdim;
    &vector.as_slice()[offset..offset + subdim]
}

fn squared_l2(lhs: &[f32], rhs: &[f32]) -> f64 {
    lhs.iter()
        .zip(rhs)
        .map(|(left, right)| {
            let delta = f64::from(*left) - f64::from(*right);
            delta * delta
        })
        .sum()
}

fn clustered_vector(seed: usize, scale: usize, jitter: f32) -> VectorValue {
    let clusters = cluster_count(scale);
    let cluster = seed % clusters;
    let ordinal = seed / clusters;
    let components = (0..DIMENSION)
        .map(|dim| {
            let base = ((((cluster + 1) * (dim + 3)) % 97) as f32 - 48.0) / 48.0;
            let local = ((((ordinal + 5) * (dim + 11)) % 31) as f32 - 15.0) * 0.001;
            base + local + jitter
        })
        .collect::<Vec<_>>();
    VectorValue::new(components).expect("benchmark vector is finite and non-empty")
}

fn cluster_count(scale: usize) -> usize {
    (scale / 250).clamp(4, 64)
}

criterion_group! {
    name = vector_pq;
    config = common::criterion_config();
    targets = bench_pq_candidate_recall
}
criterion_main!(vector_pq);
