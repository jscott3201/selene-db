#![allow(missing_docs)]
//! Criterion benches for IVF-prefiltered product-quantized vector reranking.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod vector_pq_support;

use std::mem::size_of;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{VectorTopK, VectorValue};
use vector_pq_support::{
    BinaryQuantIndex, BinaryQuantVariant, DIMENSION, K, PqCorpus, PqIndex, PqVariant,
    cluster_count, compact_count, memory_suffix, squared_l2, vector_scales,
};

const VARIANTS: [IvfPqVariant; 4] = [
    IvfPqVariant {
        name: "m16_k64_c256_p1",
        pq: PqVariant {
            name: "m16_k64_c256",
            subvectors: 16,
            codewords: 64,
            candidates: 256,
        },
        probes: 1,
    },
    IvfPqVariant {
        name: "m16_k64_c256_p2",
        pq: PqVariant {
            name: "m16_k64_c256",
            subvectors: 16,
            codewords: 64,
            candidates: 256,
        },
        probes: 2,
    },
    IvfPqVariant {
        name: "m16_k64_c1024_p1",
        pq: PqVariant {
            name: "m16_k64_c1024",
            subvectors: 16,
            codewords: 64,
            candidates: 1024,
        },
        probes: 1,
    },
    IvfPqVariant {
        name: "m16_k64_c1024_p2",
        pq: PqVariant {
            name: "m16_k64_c1024",
            subvectors: 16,
            codewords: 64,
            candidates: 1024,
        },
        probes: 2,
    },
];

const BINARY_VARIANTS: [IvfBinaryVariant; 4] = [
    IvfBinaryVariant {
        name: "sign_c64_p1",
        binary: BinaryQuantVariant {
            name: "sign_c64",
            candidates: 64,
        },
        probes: 1,
    },
    IvfBinaryVariant {
        name: "sign_c256_p1",
        binary: BinaryQuantVariant {
            name: "sign_c256",
            candidates: 256,
        },
        probes: 1,
    },
    IvfBinaryVariant {
        name: "sign_c256_p2",
        binary: BinaryQuantVariant {
            name: "sign_c256",
            candidates: 256,
        },
        probes: 2,
    },
    IvfBinaryVariant {
        name: "sign_c1024_p1",
        binary: BinaryQuantVariant {
            name: "sign_c1024",
            candidates: 1024,
        },
        probes: 1,
    },
];

#[derive(Clone, Copy, Debug)]
struct IvfPqVariant {
    name: &'static str,
    pq: PqVariant,
    probes: usize,
}

#[derive(Clone, Copy, Debug)]
struct IvfBinaryVariant {
    name: &'static str,
    binary: BinaryQuantVariant,
    probes: usize,
}

fn bench_ivf_pq_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_pq_candidate_recall");
    for scale in vector_scales() {
        for variant in VARIANTS {
            let fixture = IvfPqFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_l2",
                    format!(
                        "{}_d{DIMENSION}_k{K}_recallbp{}_rows{}_{}",
                        variant.name,
                        fixture.recall_basis_points(),
                        compact_count(fixture.searched_rows()),
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

fn bench_ivf_binary_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_binary_candidate_recall");
    for scale in vector_scales() {
        for variant in BINARY_VARIANTS {
            let fixture = IvfBinaryFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_l2",
                    format!(
                        "{}_d{DIMENSION}_k{K}_recallbp{}_rows{}_{}",
                        variant.name,
                        fixture.recall_basis_points(),
                        compact_count(fixture.searched_rows()),
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
struct IvfPqFixture {
    variant: IvfPqVariant,
    corpus: PqCorpus,
    pq: PqIndex,
    coarse: CoarsePartition,
}

impl IvfPqFixture {
    fn build(scale: usize, variant: IvfPqVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let pq = PqIndex::train(&corpus.vectors, variant.pq);
        let coarse = CoarsePartition::build(&corpus);
        Self {
            variant,
            corpus,
            pq,
            coarse,
        }
    }

    fn total_overlap(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus.total_overlap(|query| {
            self.coarse
                .candidate_rows(query, self.variant.probes, &mut rows);
            self.pq
                .search_rows(&self.corpus.vectors, query, rows.iter().copied(), K)
        })
    }

    fn recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.total_overlap())
    }

    fn searched_rows(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus
            .queries
            .iter()
            .map(|query| {
                self.coarse
                    .candidate_rows(query, self.variant.probes, &mut rows);
                rows.len()
            })
            .sum()
    }

    fn memory_suffix(&self) -> String {
        memory_suffix(
            self.pq
                .estimated_bytes()
                .saturating_add(self.coarse.estimated_bytes()),
            self.corpus.full_vector_bytes(),
        )
    }
}

#[derive(Debug)]
struct IvfBinaryFixture {
    variant: IvfBinaryVariant,
    corpus: PqCorpus,
    binary: BinaryQuantIndex,
    coarse: CoarsePartition,
}

impl IvfBinaryFixture {
    fn build(scale: usize, variant: IvfBinaryVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let binary = BinaryQuantIndex::build(&corpus.vectors, variant.binary);
        let coarse = CoarsePartition::build(&corpus);
        Self {
            variant,
            corpus,
            binary,
            coarse,
        }
    }

    fn total_overlap(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus.total_overlap(|query| {
            self.coarse
                .candidate_rows(query, self.variant.probes, &mut rows);
            self.binary
                .search_rows(&self.corpus.vectors, query, rows.iter().copied(), K)
        })
    }

    fn recall_basis_points(&self) -> usize {
        self.corpus.recall_basis_points(self.total_overlap())
    }

    fn searched_rows(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus
            .queries
            .iter()
            .map(|query| {
                self.coarse
                    .candidate_rows(query, self.variant.probes, &mut rows);
                rows.len()
            })
            .sum()
    }

    fn memory_suffix(&self) -> String {
        memory_suffix(
            self.binary
                .estimated_bytes()
                .saturating_add(self.coarse.estimated_bytes()),
            self.corpus.full_vector_bytes(),
        )
    }
}

#[derive(Debug)]
struct CoarsePartition {
    centroids: Vec<f32>,
    lists: Vec<Vec<usize>>,
}

impl CoarsePartition {
    fn build(corpus: &PqCorpus) -> Self {
        let partitions = cluster_count(corpus.scale);
        let mut sums = vec![0.0f64; partitions * DIMENSION];
        let mut counts = vec![0usize; partitions];
        let mut lists = (0..partitions).map(|_| Vec::new()).collect::<Vec<_>>();
        for (row, vector) in corpus.vectors.iter().enumerate() {
            let partition = row % partitions;
            lists[partition].push(row);
            counts[partition] += 1;
            for dim in 0..DIMENSION {
                sums[partition * DIMENSION + dim] += f64::from(vector.as_slice()[dim]);
            }
        }

        let mut centroids = vec![0.0f32; partitions * DIMENSION];
        for partition in 0..partitions {
            let inverse = 1.0 / counts[partition] as f64;
            for dim in 0..DIMENSION {
                centroids[partition * DIMENSION + dim] =
                    (sums[partition * DIMENSION + dim] * inverse) as f32;
            }
        }
        Self { centroids, lists }
    }

    fn candidate_rows(&self, query: &VectorValue, probes: usize, rows: &mut Vec<usize>) {
        let mut centroid_top_k = VectorTopK::new(probes.max(1).min(self.lists.len()));
        for centroid in 0..self.lists.len() {
            centroid_top_k.push_distance(centroid, self.centroid_distance(query, centroid));
        }

        rows.clear();
        for centroid in centroid_top_k.into_hits() {
            rows.extend_from_slice(&self.lists[centroid.key]);
        }
    }

    fn estimated_bytes(&self) -> usize {
        let list_rows = self.lists.iter().map(Vec::len).sum::<usize>();
        self.centroids
            .len()
            .saturating_mul(size_of::<f32>())
            .saturating_add(list_rows.saturating_mul(size_of::<usize>()))
    }

    fn centroid_distance(&self, query: &VectorValue, centroid: usize) -> f64 {
        let offset = centroid * DIMENSION;
        squared_l2(
            query.as_slice(),
            &self.centroids[offset..offset + DIMENSION],
        )
    }
}

criterion_group! {
    name = vector_ivf_pq;
    config = common::criterion_config();
    targets = bench_ivf_pq_candidate_recall, bench_ivf_binary_candidate_recall
}
criterion_main!(vector_ivf_pq);
