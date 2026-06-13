use std::{num::NonZeroUsize, sync::Arc};

use criterion::{BenchmarkGroup, BenchmarkId, measurement::WallTime};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};

use super::fixture::{OmlxGqlQueryRootFixture, TOP_K};

#[derive(Clone, Copy)]
struct JsonScoreQuality {
    precision: usize,
    current_precision: usize,
    target_hit: Option<usize>,
}

pub(super) fn bench_json_score_rows(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
) {
    let vector_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let text_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    fixture.warm_query_root_json_current_vector_batch_cache(registry, Arc::clone(&vector_cache));
    fixture.warm_query_root_json_current_text_batch_cache(registry, Arc::clone(&text_cache));
    let vector_quality = JsonScoreQuality {
        precision: fixture.gql_json_current_vector_batch_precision_basis_points(
            registry,
            Some(Arc::clone(&vector_cache)),
        ),
        current_precision: fixture.gql_json_current_vector_batch_current_precision_basis_points(
            registry,
            Some(Arc::clone(&vector_cache)),
        ),
        target_hit: fixture.gql_json_current_vector_batch_target_hit_basis_points(
            registry,
            Some(Arc::clone(&vector_cache)),
        ),
    };
    let text_quality = JsonScoreQuality {
        precision: fixture.gql_json_current_text_batch_precision_basis_points(
            registry,
            Some(Arc::clone(&text_cache)),
        ),
        current_precision: fixture.gql_json_current_text_batch_current_precision_basis_points(
            registry,
            Some(Arc::clone(&text_cache)),
        ),
        target_hit: fixture.gql_json_current_text_batch_target_hit_basis_points(
            registry,
            Some(Arc::clone(&text_cache)),
        ),
    };
    bench_json_vector_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        vector_quality,
        vector_cache,
    );
    bench_json_text_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        text_quality,
        text_cache,
    );
}

fn bench_json_vector_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: JsonScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_json_current_vector_batch",
            row_label(fixture, model_id, corpus_label, quality),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_json_current_vector_batch_query(registry, Some(Arc::clone(&cache)))
                        .row_count(),
                );
            });
        },
    );
}

fn bench_json_text_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: JsonScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_json_current_text_score_batch",
            row_label(fixture, model_id, corpus_label, quality),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_json_current_text_batch_query(registry, Some(Arc::clone(&cache)))
                        .row_count(),
                );
            });
        },
    );
}

fn row_label(
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: JsonScoreQuality,
) -> String {
    let mut label = format!(
        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}_curbp{}",
        model_id,
        corpus_label,
        fixture.query_count(),
        TOP_K,
        fixture.first_query_root_count(),
        fixture.first_query_current_state_intersection_count(),
        fixture.dimension,
        quality.precision,
        quality.current_precision,
    );
    if let Some(target_hit) = quality.target_hit {
        label.push_str(&format!("_hitbp{target_hit}"));
    }
    label
}
