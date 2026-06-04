//! Task/session graph filter rows for graph/vector retrieval benchmarks.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::common::scale_label;

use super::support::{FACTS_PER_TOPIC, RESULT_K, SEED_K, WIDE_SEED_K, basis_points, vector_scales};
use super::{MemoryRetrievalFixture, TopologyNoise};

mod maintenance;
mod selection;

const SESSION_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::NoisyWcc,
    SessionStrategy::LabelPropagation,
    SessionStrategy::GraphSessionFilter,
    SessionStrategy::GraphSessionCurrentFilter,
    SessionStrategy::GraphSessionUnsupersededFilter,
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpandK1,
    SessionStrategy::GraphSessionProvenanceExpand,
    SessionStrategy::GraphScopeFilter,
    SessionStrategy::GraphScopeCurrentFilter,
    SessionStrategy::GraphScopeUnsupersededFilter,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpandK1,
    SessionStrategy::GraphScopeProvenanceExpand,
    SessionStrategy::TopicFilter,
];

const PROVENANCE_PRESSURE_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpandK1,
    SessionStrategy::GraphSessionProvenanceExpand,
    SessionStrategy::GraphSessionProvenanceExpandK8,
    SessionStrategy::GraphSessionProvenanceExpandK16,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpandK1,
    SessionStrategy::GraphScopeProvenanceExpand,
    SessionStrategy::GraphScopeProvenanceExpandK8,
    SessionStrategy::GraphScopeProvenanceExpandK16,
    SessionStrategy::TopicFilter,
];

const MULTIHOP_PROVENANCE_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpandK1,
    SessionStrategy::GraphSessionProvenanceExpand2HopK1,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpandK1,
    SessionStrategy::GraphScopeProvenanceExpand2HopK1,
    SessionStrategy::TopicFilter,
];

const SPARSE_MULTIHOP_PROVENANCE_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpandK1,
    SessionStrategy::GraphSessionProvenanceExpand2HopK1,
    SessionStrategy::GraphSessionProvenanceExpand2Hop,
    SessionStrategy::GraphSessionProvenanceExpand2HopK8,
    SessionStrategy::GraphSessionProvenanceExpandK16,
    SessionStrategy::GraphSessionProvenanceExpand2HopK16,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpandK1,
    SessionStrategy::GraphScopeProvenanceExpand2HopK1,
    SessionStrategy::GraphScopeProvenanceExpand2Hop,
    SessionStrategy::GraphScopeProvenanceExpand2HopK8,
    SessionStrategy::GraphScopeProvenanceExpandK16,
    SessionStrategy::GraphScopeProvenanceExpand2HopK16,
    SessionStrategy::TopicFilter,
];

const ADAPTIVE_PROVENANCE_PLANS: &[(usize, usize)] = &[
    (1, 1),
    (1, 2),
    (SEED_K, 2),
    (SEED_K * 2, 2),
    (WIDE_SEED_K, 2),
];

const ADAPTIVE_PROVENANCE_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpand2HopK16,
    SessionStrategy::GraphSessionProvenanceAdaptiveQuality,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpand2HopK16,
    SessionStrategy::GraphScopeProvenanceAdaptiveQuality,
    SessionStrategy::TopicFilter,
];

const NEGATIVE_EVIDENCE_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionUnresolvedCurrentFilter,
    SessionStrategy::GraphSessionMaterializedUnresolvedCurrentFilter,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeUnresolvedCurrentFilter,
    SessionStrategy::GraphScopeMaterializedUnresolvedCurrentFilter,
    SessionStrategy::TopicFilter,
];

const ACTIVE_SUBGRAPH_COMPOSITION_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedCurrentFilter,
    SessionStrategy::GraphSessionMaterializedUnresolvedCurrentFilter,
    SessionStrategy::GraphSessionProvenanceExpand2HopK16,
    SessionStrategy::GraphSessionUnresolvedProvenanceExpand2HopK16,
    SessionStrategy::GraphScopeMaterializedCurrentFilter,
    SessionStrategy::GraphScopeMaterializedUnresolvedCurrentFilter,
    SessionStrategy::GraphScopeProvenanceExpand2HopK16,
    SessionStrategy::GraphScopeUnresolvedProvenanceExpand2HopK16,
    SessionStrategy::TopicFilter,
];

const ACTIVE_SUBGRAPH_FALLBACK_STRATEGIES: &[SessionStrategy] = &[
    SessionStrategy::GraphSessionMaterializedUnresolvedCurrentFilter,
    SessionStrategy::GraphSessionUnresolvedProvenanceExpand2HopK16,
    SessionStrategy::GraphSessionUnresolvedProvenanceFallback2HopK16,
    SessionStrategy::GraphScopeMaterializedUnresolvedCurrentFilter,
    SessionStrategy::GraphScopeUnresolvedProvenanceExpand2HopK16,
    SessionStrategy::GraphScopeUnresolvedProvenanceFallback2HopK16,
    SessionStrategy::TopicFilter,
];

#[derive(Clone, Copy, Debug)]
enum SessionStrategy {
    NoisyWcc,
    LabelPropagation,
    GraphSessionFilter,
    GraphSessionCurrentFilter,
    GraphSessionUnsupersededFilter,
    GraphSessionMaterializedCurrentFilter,
    GraphSessionUnresolvedCurrentFilter,
    GraphSessionMaterializedUnresolvedCurrentFilter,
    GraphSessionProvenanceExpandK1,
    GraphSessionProvenanceExpand2HopK1,
    GraphSessionProvenanceExpand,
    GraphSessionProvenanceExpand2Hop,
    GraphSessionProvenanceExpandK8,
    GraphSessionProvenanceExpand2HopK8,
    GraphSessionProvenanceExpandK16,
    GraphSessionProvenanceExpand2HopK16,
    GraphSessionProvenanceAdaptiveQuality,
    GraphSessionUnresolvedProvenanceExpand2HopK16,
    GraphSessionUnresolvedProvenanceFallback2HopK16,
    GraphScopeFilter,
    GraphScopeCurrentFilter,
    GraphScopeUnsupersededFilter,
    GraphScopeMaterializedCurrentFilter,
    GraphScopeUnresolvedCurrentFilter,
    GraphScopeMaterializedUnresolvedCurrentFilter,
    GraphScopeProvenanceExpandK1,
    GraphScopeProvenanceExpand2HopK1,
    GraphScopeProvenanceExpand,
    GraphScopeProvenanceExpand2Hop,
    GraphScopeProvenanceExpandK8,
    GraphScopeProvenanceExpand2HopK8,
    GraphScopeProvenanceExpandK16,
    GraphScopeProvenanceExpand2HopK16,
    GraphScopeProvenanceAdaptiveQuality,
    GraphScopeUnresolvedProvenanceExpand2HopK16,
    GraphScopeUnresolvedProvenanceFallback2HopK16,
    TopicFilter,
}

impl SessionStrategy {
    const fn name(self) -> &'static str {
        match self {
            Self::NoisyWcc => "noisy_wcc",
            Self::LabelPropagation => "label_propagation",
            Self::GraphSessionFilter => "graph_session_filter",
            Self::GraphSessionCurrentFilter => "graph_session_current_filter",
            Self::GraphSessionUnsupersededFilter => "graph_session_unsuperseded_filter",
            Self::GraphSessionMaterializedCurrentFilter => {
                "graph_session_materialized_current_filter"
            }
            Self::GraphSessionUnresolvedCurrentFilter => "graph_session_unresolved_current_filter",
            Self::GraphSessionMaterializedUnresolvedCurrentFilter => {
                "graph_session_materialized_unresolved_current_filter"
            }
            Self::GraphSessionProvenanceExpandK1 => "graph_session_provenance_expand_k1",
            Self::GraphSessionProvenanceExpand2HopK1 => "graph_session_provenance_expand_2hop_k1",
            Self::GraphSessionProvenanceExpand => "graph_session_provenance_expand",
            Self::GraphSessionProvenanceExpand2Hop => "graph_session_provenance_expand_2hop",
            Self::GraphSessionProvenanceExpandK8 => "graph_session_provenance_expand_k8",
            Self::GraphSessionProvenanceExpand2HopK8 => "graph_session_provenance_expand_2hop_k8",
            Self::GraphSessionProvenanceExpandK16 => "graph_session_provenance_expand_k16",
            Self::GraphSessionProvenanceExpand2HopK16 => "graph_session_provenance_expand_2hop_k16",
            Self::GraphSessionProvenanceAdaptiveQuality => {
                "graph_session_provenance_adaptive_quality"
            }
            Self::GraphSessionUnresolvedProvenanceExpand2HopK16 => {
                "graph_session_unresolved_provenance_expand_2hop_k16"
            }
            Self::GraphSessionUnresolvedProvenanceFallback2HopK16 => {
                "graph_session_unresolved_provenance_fallback_2hop_k16"
            }
            Self::GraphScopeFilter => "graph_scope_filter",
            Self::GraphScopeCurrentFilter => "graph_scope_current_filter",
            Self::GraphScopeUnsupersededFilter => "graph_scope_unsuperseded_filter",
            Self::GraphScopeMaterializedCurrentFilter => "graph_scope_materialized_current_filter",
            Self::GraphScopeUnresolvedCurrentFilter => "graph_scope_unresolved_current_filter",
            Self::GraphScopeMaterializedUnresolvedCurrentFilter => {
                "graph_scope_materialized_unresolved_current_filter"
            }
            Self::GraphScopeProvenanceExpandK1 => "graph_scope_provenance_expand_k1",
            Self::GraphScopeProvenanceExpand2HopK1 => "graph_scope_provenance_expand_2hop_k1",
            Self::GraphScopeProvenanceExpand => "graph_scope_provenance_expand",
            Self::GraphScopeProvenanceExpand2Hop => "graph_scope_provenance_expand_2hop",
            Self::GraphScopeProvenanceExpandK8 => "graph_scope_provenance_expand_k8",
            Self::GraphScopeProvenanceExpand2HopK8 => "graph_scope_provenance_expand_2hop_k8",
            Self::GraphScopeProvenanceExpandK16 => "graph_scope_provenance_expand_k16",
            Self::GraphScopeProvenanceExpand2HopK16 => "graph_scope_provenance_expand_2hop_k16",
            Self::GraphScopeProvenanceAdaptiveQuality => "graph_scope_provenance_adaptive_quality",
            Self::GraphScopeUnresolvedProvenanceExpand2HopK16 => {
                "graph_scope_unresolved_provenance_expand_2hop_k16"
            }
            Self::GraphScopeUnresolvedProvenanceFallback2HopK16 => {
                "graph_scope_unresolved_provenance_fallback_2hop_k16"
            }
            Self::TopicFilter => "topic_filter",
        }
    }

    const fn provenance_plan(self) -> Option<(usize, usize)> {
        match self {
            Self::GraphSessionProvenanceExpandK1 | Self::GraphScopeProvenanceExpandK1 => {
                Some((1, 1))
            }
            Self::GraphSessionProvenanceExpand2HopK1 | Self::GraphScopeProvenanceExpand2HopK1 => {
                Some((1, 2))
            }
            Self::GraphSessionProvenanceExpand | Self::GraphScopeProvenanceExpand => {
                Some((SEED_K, 1))
            }
            Self::GraphSessionProvenanceExpand2Hop | Self::GraphScopeProvenanceExpand2Hop => {
                Some((SEED_K, 2))
            }
            Self::GraphSessionProvenanceExpandK8 | Self::GraphScopeProvenanceExpandK8 => {
                Some((SEED_K * 2, 1))
            }
            Self::GraphSessionProvenanceExpand2HopK8 | Self::GraphScopeProvenanceExpand2HopK8 => {
                Some((SEED_K * 2, 2))
            }
            Self::GraphSessionProvenanceExpandK16 | Self::GraphScopeProvenanceExpandK16 => {
                Some((WIDE_SEED_K, 1))
            }
            Self::GraphSessionProvenanceExpand2HopK16 | Self::GraphScopeProvenanceExpand2HopK16 => {
                Some((WIDE_SEED_K, 2))
            }
            Self::GraphSessionUnresolvedProvenanceExpand2HopK16
            | Self::GraphSessionUnresolvedProvenanceFallback2HopK16
            | Self::GraphScopeUnresolvedProvenanceExpand2HopK16
            | Self::GraphScopeUnresolvedProvenanceFallback2HopK16 => Some((WIDE_SEED_K, 2)),
            _ => None,
        }
    }

    const fn is_adaptive_provenance(self) -> bool {
        matches!(
            self,
            Self::GraphSessionProvenanceAdaptiveQuality | Self::GraphScopeProvenanceAdaptiveQuality
        )
    }

    const fn is_unresolved_provenance(self) -> bool {
        matches!(
            self,
            Self::GraphSessionUnresolvedProvenanceExpand2HopK16
                | Self::GraphScopeUnresolvedProvenanceExpand2HopK16
        )
    }

    const fn is_unresolved_provenance_fallback(self) -> bool {
        matches!(
            self,
            Self::GraphSessionUnresolvedProvenanceFallback2HopK16
                | Self::GraphScopeUnresolvedProvenanceFallback2HopK16
        )
    }
}

pub(super) fn bench(c: &mut Criterion) {
    bench_session_filter_pressure(c);
    bench_sparse_provenance_pressure(c);
    bench_noisy_sparse_provenance_pressure(c);
    bench_multihop_provenance_pressure(c);
    bench_noisy_multihop_provenance_pressure(c);
    bench_noisy_sparse_multihop_provenance_pressure(c);
    bench_adaptive_provenance_pressure(c);
    bench_negative_evidence_pressure(c);
    bench_active_subgraph_composition_pressure(c);
    bench_active_subgraph_fallback_pressure(c);
    maintenance::bench_active_set_maintenance_pressure(c);
}

fn bench_session_filter_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_session_filter_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_community_topology(
            scale,
            TopologyNoise::CrossTopicSupportRing,
        );
        for &strategy in SESSION_STRATEGIES {
            let avg_candidates = fixture.average_session_candidates(strategy);
            let quality = fixture.session_quality(strategy);
            group.throughput(Throughput::Elements(
                (fixture.query_count() * avg_candidates) as u64,
            ));
            group.bench_function(
                BenchmarkId::new(
                    strategy.name(),
                    format!(
                        "{}_q{}_c{}_covbp{}_curbp{}_precbp{}",
                        scale_label(fixture.scale()),
                        fixture.query_count(),
                        avg_candidates,
                        basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
                        basis_points(
                            quality.current_coverage,
                            fixture.query_count() * FACTS_PER_TOPIC
                        ),
                        basis_points(quality.precision, fixture.query_count() * RESULT_K),
                    ),
                ),
                |b| {
                    b.iter(|| {
                        black_box(fixture.session_total_coverage(strategy));
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_sparse_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_sparse_provenance_pressure");
    for scale in vector_scales() {
        let fixture =
            MemoryRetrievalFixture::build_with_topology(scale, TopologyNoise::SparseSupport);
        for &strategy in PROVENANCE_PRESSURE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_noisy_sparse_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_noisy_sparse_provenance_pressure");
    for scale in vector_scales() {
        let fixture =
            MemoryRetrievalFixture::build_with_topology(scale, TopologyNoise::NoisySparseSupport);
        for &strategy in PROVENANCE_PRESSURE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_multihop_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_multihop_provenance_pressure");
    for scale in vector_scales() {
        let fixture =
            MemoryRetrievalFixture::build_with_topology(scale, TopologyNoise::MultiHopSupport);
        for &strategy in MULTIHOP_PROVENANCE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_noisy_multihop_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_noisy_multihop_provenance_pressure");
    for scale in vector_scales() {
        let fixture =
            MemoryRetrievalFixture::build_with_topology(scale, TopologyNoise::NoisyMultiHopSupport);
        for &strategy in MULTIHOP_PROVENANCE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_noisy_sparse_multihop_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_noisy_sparse_multihop_provenance_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopSupport,
        );
        for &strategy in SPARSE_MULTIHOP_PROVENANCE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_adaptive_provenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_adaptive_provenance_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopSupport,
        );
        for &strategy in ADAPTIVE_PROVENANCE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_negative_evidence_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_negative_evidence_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::ContradictedCurrentDuplicates,
        );
        for &strategy in NEGATIVE_EVIDENCE_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_active_subgraph_composition_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_active_subgraph_composition_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopContradicted,
        );
        for &strategy in ACTIVE_SUBGRAPH_COMPOSITION_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_active_subgraph_fallback_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_active_subgraph_fallback_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::NoisySparseMultiHopContradicted,
        );
        for &strategy in ACTIVE_SUBGRAPH_FALLBACK_STRATEGIES {
            bench_strategy(&mut group, &fixture, strategy);
        }
    }
    group.finish();
}

fn bench_strategy(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &MemoryRetrievalFixture,
    strategy: SessionStrategy,
) {
    let avg_candidates = fixture.average_session_candidates(strategy);
    let quality = fixture.session_quality(strategy);
    group.throughput(Throughput::Elements(
        (fixture.query_count() * avg_candidates) as u64,
    ));
    group.bench_function(
        BenchmarkId::new(
            strategy.name(),
            format!(
                "{}_q{}_c{}_covbp{}_curbp{}_precbp{}",
                scale_label(fixture.scale()),
                fixture.query_count(),
                avg_candidates,
                basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
                basis_points(
                    quality.current_coverage,
                    fixture.query_count() * FACTS_PER_TOPIC
                ),
                basis_points(quality.precision, fixture.query_count() * RESULT_K),
            ),
        ),
        |b| {
            b.iter(|| {
                black_box(fixture.session_total_coverage(strategy));
            });
        },
    );
}
