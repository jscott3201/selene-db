#![allow(missing_docs)]
//! Criterion benches for IVF-prefiltered product-quantized vector reranking.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod vector_pq_support;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vector_pq_support::ivf::{CoarsePartition, IvfTurboQuantFixture, IvfTurboQuantVariant};
use vector_pq_support::turbo_quant::{
    TurboQuantCalibration, TurboQuantCodebook, TurboQuantScorer, TurboQuantVariant,
};
use vector_pq_support::{
    BinaryQuantIndex, BinaryQuantVariant, CorpusProfile, DIMENSION, K, PqCorpus, PqIndex,
    PqVariant, ScalarQuantIndex, ScalarQuantVariant, compact_count, memory_suffix, vector_scales,
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

const SCALAR_CODE_VARIANTS: [IvfScalarCodeVariant; 4] = [
    IvfScalarCodeVariant {
        name: "u8code_c64_p1",
        scalar: ScalarQuantVariant {
            name: "u8code_c64",
            candidates: 64,
        },
        probes: 1,
    },
    IvfScalarCodeVariant {
        name: "u8code_c256_p1",
        scalar: ScalarQuantVariant {
            name: "u8code_c256",
            candidates: 256,
        },
        probes: 1,
    },
    IvfScalarCodeVariant {
        name: "u8code_c256_p2",
        scalar: ScalarQuantVariant {
            name: "u8code_c256",
            candidates: 256,
        },
        probes: 2,
    },
    IvfScalarCodeVariant {
        name: "u8code_c1024_p1",
        scalar: ScalarQuantVariant {
            name: "u8code_c1024",
            candidates: 1024,
        },
        probes: 1,
    },
];

const TURBO_QUANT_VARIANTS: [IvfTurboQuantVariant; 3] = [
    IvfTurboQuantVariant {
        name: "tqplus4lut_c256_p1",
        turbo: TurboQuantVariant {
            name: "tqplus4lut_c256",
            bit_width: 4,
            candidates: 256,
            codebook: TurboQuantCodebook::NormalLloydMax,
            calibration: TurboQuantCalibration::Quantile,
            scorer: TurboQuantScorer::ByteLut,
        },
        probes: 1,
    },
    IvfTurboQuantVariant {
        name: "tqplus4lut_c1024_p1",
        turbo: TurboQuantVariant {
            name: "tqplus4lut_c1024",
            bit_width: 4,
            candidates: 1024,
            codebook: TurboQuantCodebook::NormalLloydMax,
            calibration: TurboQuantCalibration::Quantile,
            scorer: TurboQuantScorer::ByteLut,
        },
        probes: 1,
    },
    IvfTurboQuantVariant {
        name: "tqplus4lut_c1024_p4",
        turbo: TurboQuantVariant {
            name: "tqplus4lut_c1024",
            bit_width: 4,
            candidates: 1024,
            codebook: TurboQuantCodebook::NormalLloydMax,
            calibration: TurboQuantCalibration::Quantile,
            scorer: TurboQuantScorer::ByteLut,
        },
        probes: 4,
    },
];

const OVERLAP_PQ_VARIANTS: [IvfPqVariant; 3] = [
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
        name: "m16_k64_c1024_p4",
        pq: PqVariant {
            name: "m16_k64_c1024",
            subvectors: 16,
            codewords: 64,
            candidates: 1024,
        },
        probes: 4,
    },
    IvfPqVariant {
        name: "m16_k64_c4096_p4",
        pq: PqVariant {
            name: "m16_k64_c4096",
            subvectors: 16,
            codewords: 64,
            candidates: 4096,
        },
        probes: 4,
    },
];

const OVERLAP_BINARY_VARIANTS: [IvfBinaryVariant; 4] = [
    IvfBinaryVariant {
        name: "sign_c256_p1",
        binary: BinaryQuantVariant {
            name: "sign_c256",
            candidates: 256,
        },
        probes: 1,
    },
    IvfBinaryVariant {
        name: "sign_c256_p4",
        binary: BinaryQuantVariant {
            name: "sign_c256",
            candidates: 256,
        },
        probes: 4,
    },
    IvfBinaryVariant {
        name: "sign_c1024_p1",
        binary: BinaryQuantVariant {
            name: "sign_c1024",
            candidates: 1024,
        },
        probes: 1,
    },
    IvfBinaryVariant {
        name: "sign_c1024_p4",
        binary: BinaryQuantVariant {
            name: "sign_c1024",
            candidates: 1024,
        },
        probes: 4,
    },
];

const OVERLAP_TURBO_QUANT_VARIANTS: [IvfTurboQuantVariant; 2] = [
    IvfTurboQuantVariant {
        name: "tqplus4lut_c1024_p1",
        turbo: TurboQuantVariant {
            name: "tqplus4lut_c1024",
            bit_width: 4,
            candidates: 1024,
            codebook: TurboQuantCodebook::NormalLloydMax,
            calibration: TurboQuantCalibration::Quantile,
            scorer: TurboQuantScorer::ByteLut,
        },
        probes: 1,
    },
    IvfTurboQuantVariant {
        name: "tqplus4lut_c1024_p4",
        turbo: TurboQuantVariant {
            name: "tqplus4lut_c1024",
            bit_width: 4,
            candidates: 1024,
            codebook: TurboQuantCodebook::NormalLloydMax,
            calibration: TurboQuantCalibration::Quantile,
            scorer: TurboQuantScorer::ByteLut,
        },
        probes: 4,
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

#[derive(Clone, Copy, Debug)]
struct IvfScalarCodeVariant {
    name: &'static str,
    scalar: ScalarQuantVariant,
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

fn bench_ivf_scalar_code_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_scalar_code_candidate_recall");
    for scale in vector_scales() {
        for variant in SCALAR_CODE_VARIANTS {
            let fixture = IvfScalarCodeFixture::build(scale, variant);
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

fn bench_ivf_turbo_quant_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_turbo_quant_candidate_recall");
    for scale in vector_scales() {
        for variant in TURBO_QUANT_VARIANTS {
            let fixture = IvfTurboQuantFixture::build(scale, variant);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "cluster_cos",
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

fn bench_ivf_overlap_candidate_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_overlap_candidate_recall");
    for scale in vector_scales() {
        for variant in OVERLAP_PQ_VARIANTS {
            let fixture = IvfPqFixture::build_with_profile(scale, variant, CorpusProfile::Overlap);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "pq_overlap_l2",
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
        for variant in OVERLAP_BINARY_VARIANTS {
            let fixture =
                IvfBinaryFixture::build_with_profile(scale, variant, CorpusProfile::Overlap);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "binary_overlap_l2",
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
        for variant in OVERLAP_TURBO_QUANT_VARIANTS {
            let fixture =
                IvfTurboQuantFixture::build_with_profile(scale, variant, CorpusProfile::Overlap);
            group.throughput(Throughput::Elements(
                u64::try_from(fixture.searched_rows()).unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    "turbo_overlap_cos",
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
        Self::build_with_profile(scale, variant, CorpusProfile::Clustered)
    }

    fn build_with_profile(scale: usize, variant: IvfPqVariant, profile: CorpusProfile) -> Self {
        let corpus = PqCorpus::build_profile(scale, profile);
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
        Self::build_with_profile(scale, variant, CorpusProfile::Clustered)
    }

    fn build_with_profile(scale: usize, variant: IvfBinaryVariant, profile: CorpusProfile) -> Self {
        let corpus = PqCorpus::build_profile(scale, profile);
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
struct IvfScalarCodeFixture {
    variant: IvfScalarCodeVariant,
    corpus: PqCorpus,
    scalar: ScalarQuantIndex,
    coarse: CoarsePartition,
}

impl IvfScalarCodeFixture {
    fn build(scale: usize, variant: IvfScalarCodeVariant) -> Self {
        let corpus = PqCorpus::build(scale);
        let scalar = ScalarQuantIndex::build(&corpus.vectors, variant.scalar);
        let coarse = CoarsePartition::build(&corpus);
        Self {
            variant,
            corpus,
            scalar,
            coarse,
        }
    }

    fn total_overlap(&self) -> usize {
        let mut rows = Vec::new();
        self.corpus.total_overlap(|query| {
            self.coarse
                .candidate_rows(query, self.variant.probes, &mut rows);
            self.scalar
                .search_rows_code_l2(&self.corpus.vectors, query, rows.iter().copied(), K)
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
            self.scalar
                .estimated_bytes()
                .saturating_add(self.coarse.estimated_bytes()),
            self.corpus.full_vector_bytes(),
        )
    }
}

criterion_group! {
    name = vector_ivf_pq;
    config = common::criterion_config();
    targets =
        bench_ivf_pq_candidate_recall,
        bench_ivf_binary_candidate_recall,
        bench_ivf_scalar_code_candidate_recall,
        bench_ivf_turbo_quant_candidate_recall,
        bench_ivf_overlap_candidate_recall
}
criterion_main!(vector_ivf_pq);
