use std::{num::NonZeroUsize, sync::Arc};

use criterion::{BenchmarkGroup, BenchmarkId, measurement::WallTime};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};

use super::fixture::{OmlxGqlQueryRootFixture, TOP_K};

pub(super) fn bench_state_batch_rows(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    current_precision: usize,
) {
    let current_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let provenance_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    fixture.warm_query_root_current_state_batch_cache(registry, Arc::clone(&current_cache));
    fixture.warm_query_root_provenance_state_batch_cache(registry, Arc::clone(&provenance_cache));
    let current_state_precision = fixture.gql_current_state_batch_current_precision_basis_points(
        registry,
        Some(Arc::clone(&current_cache)),
    );
    let provenance_state_precision = fixture
        .gql_provenance_state_batch_current_precision_basis_points(
            registry,
            Some(Arc::clone(&provenance_cache)),
        );
    let current_target_hit = fixture.gql_current_state_batch_target_hit_basis_points(
        registry,
        Some(Arc::clone(&current_cache)),
    );
    let provenance_target_hit = fixture.gql_provenance_state_batch_target_hit_basis_points(
        registry,
        Some(Arc::clone(&provenance_cache)),
    );
    bench_current_state_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        current_precision,
        current_state_precision,
        current_target_hit,
        current_cache,
    );
    bench_provenance_state_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        current_precision,
        provenance_state_precision,
        provenance_target_hit,
        provenance_cache,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "Criterion row labels need the measured shape"
)]
fn bench_current_state_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    current_precision: usize,
    state_precision: usize,
    target_hit: Option<usize>,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_current_state_intersection_batch",
            row_label(
                fixture,
                model_id,
                corpus_label,
                current_precision,
                state_precision,
                target_hit,
            ),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture.execute_current_state_batch_query(registry, Some(Arc::clone(&cache))),
                );
            });
        },
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "Criterion row labels need the measured shape"
)]
fn bench_provenance_state_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    current_precision: usize,
    state_precision: usize,
    target_hit: Option<usize>,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_provenance_state_intersection_batch",
            row_label(
                fixture,
                model_id,
                corpus_label,
                current_precision,
                state_precision,
                target_hit,
            ),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_provenance_state_batch_query(registry, Some(Arc::clone(&cache))),
                );
            });
        },
    );
}

fn row_label(
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    current_precision: usize,
    state_precision: usize,
    target_hit: Option<usize>,
) -> String {
    let mut label = format!(
        "{}_{}_q{}_k{}_r{}_c{}_dim{}_basecurbp{}_curbp{}",
        model_id,
        corpus_label,
        fixture.query_count(),
        TOP_K,
        fixture.first_query_root_count(),
        fixture.first_query_current_state_intersection_count(),
        fixture.dimension,
        current_precision,
        state_precision,
    );
    if let Some(target_hit) = target_hit {
        label.push_str(&format!("_hitbp{target_hit}"));
    }
    label
}
