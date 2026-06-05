use std::{num::NonZeroUsize, sync::Arc};

use criterion::{BenchmarkGroup, BenchmarkId, measurement::WallTime};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};

use super::fixture::{OmlxGqlQueryRootFixture, TOP_K};

#[derive(Clone, Copy)]
struct TextScoreQuality {
    precision: usize,
    current_precision: usize,
    target_hit: Option<usize>,
}

pub(super) fn bench_text_score_rows(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
) {
    let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let current_state_batch_cache =
        Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let current_state_text_vector_batch_cache =
        Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    let vector_text_batch_cache =
        Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
    fixture.warm_query_root_text_score_cache(registry, Arc::clone(&cache));
    fixture.warm_query_root_text_score_batch_cache(registry, Arc::clone(&batch_cache));
    fixture.warm_query_root_current_state_text_score_batch_cache(
        registry,
        Arc::clone(&current_state_batch_cache),
    );
    fixture.warm_query_root_current_state_text_vector_batch_cache(
        registry,
        Arc::clone(&current_state_text_vector_batch_cache),
    );
    fixture.warm_query_root_vector_text_batch_cache(registry, Arc::clone(&vector_text_batch_cache));
    let quality = TextScoreQuality {
        precision: fixture
            .gql_text_score_precision_basis_points(registry, Some(Arc::clone(&cache))),
        current_precision: fixture
            .gql_text_score_current_precision_basis_points(registry, Some(Arc::clone(&cache))),
        target_hit: fixture
            .gql_text_score_target_hit_basis_points(registry, Some(Arc::clone(&cache))),
    };
    let batch_quality = TextScoreQuality {
        precision: fixture
            .gql_text_score_batch_precision_basis_points(registry, Some(Arc::clone(&batch_cache))),
        current_precision: fixture.gql_text_score_batch_current_precision_basis_points(
            registry,
            Some(Arc::clone(&batch_cache)),
        ),
        target_hit: fixture
            .gql_text_score_batch_target_hit_basis_points(registry, Some(Arc::clone(&batch_cache))),
    };
    let current_state_batch_quality = TextScoreQuality {
        precision: fixture.gql_current_state_text_score_batch_precision_basis_points(
            registry,
            Some(Arc::clone(&current_state_batch_cache)),
        ),
        current_precision: fixture
            .gql_current_state_text_score_batch_current_precision_basis_points(
                registry,
                Some(Arc::clone(&current_state_batch_cache)),
            ),
        target_hit: fixture.gql_current_state_text_score_batch_target_hit_basis_points(
            registry,
            Some(Arc::clone(&current_state_batch_cache)),
        ),
    };
    let current_state_text_vector_batch_quality = TextScoreQuality {
        precision: fixture.gql_current_state_text_vector_batch_precision_basis_points(
            registry,
            Some(Arc::clone(&current_state_text_vector_batch_cache)),
        ),
        current_precision: fixture
            .gql_current_state_text_vector_batch_current_precision_basis_points(
                registry,
                Some(Arc::clone(&current_state_text_vector_batch_cache)),
            ),
        target_hit: fixture.gql_current_state_text_vector_batch_target_hit_basis_points(
            registry,
            Some(Arc::clone(&current_state_text_vector_batch_cache)),
        ),
    };
    let vector_text_batch_quality = TextScoreQuality {
        precision: fixture.gql_vector_text_batch_precision_basis_points(
            registry,
            Some(Arc::clone(&vector_text_batch_cache)),
        ),
        current_precision: fixture.gql_vector_text_batch_current_precision_basis_points(
            registry,
            Some(Arc::clone(&vector_text_batch_cache)),
        ),
        target_hit: fixture.gql_vector_text_batch_target_hit_basis_points(
            registry,
            Some(Arc::clone(&vector_text_batch_cache)),
        ),
    };
    let mut plan_session = fixture.reusable_plan_cache_session();
    fixture.warm_query_root_text_score_session(&mut plan_session, registry);
    bench_shared_cache(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        quality,
        cache,
    );
    bench_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        batch_quality,
        batch_cache,
    );
    bench_current_state_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        current_state_batch_quality,
        current_state_batch_cache,
    );
    bench_current_state_text_vector_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        current_state_text_vector_batch_quality,
        current_state_text_vector_batch_cache,
    );
    bench_vector_text_batch(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        vector_text_batch_quality,
        vector_text_batch_cache,
    );
    bench_plan_cache_session(
        group,
        registry,
        fixture,
        model_id,
        corpus_label,
        quality,
        plan_session,
    );
}

fn bench_vector_text_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_vector_text_batch",
            row_label_with_candidate_count(fixture, model_id, corpus_label, quality, TOP_K),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_vector_text_batch_query(registry, Some(Arc::clone(&cache)))
                        .row_count(),
                );
            });
        },
    );
}

fn bench_current_state_text_vector_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_current_state_text_vector_batch",
            row_label_with_candidate_count(
                fixture,
                model_id,
                corpus_label,
                quality,
                fixture.first_query_current_state_intersection_count(),
            ),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_current_state_text_vector_batch_query(
                            registry,
                            Some(Arc::clone(&cache)),
                        )
                        .row_count(),
                );
            });
        },
    );
}

fn bench_current_state_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_current_state_text_score_batch",
            row_label_with_candidate_count(
                fixture,
                model_id,
                corpus_label,
                quality,
                fixture.first_query_current_state_intersection_count(),
            ),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_current_state_text_score_batch_query(
                            registry,
                            Some(Arc::clone(&cache)),
                        )
                        .row_count(),
                );
            });
        },
    );
}

fn bench_batch(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_text_score_batch",
            row_label(fixture, model_id, corpus_label, quality),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture
                        .execute_text_score_batch_query(registry, Some(Arc::clone(&cache)))
                        .row_count(),
                );
            });
        },
    );
}

fn bench_shared_cache(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    cache: Arc<CallPlanCache>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_cache_query_root_text_score",
            row_label(fixture, model_id, corpus_label, quality),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture.execute_all_text_score_queries(registry, Some(Arc::clone(&cache))),
                );
            });
        },
    );
}

fn bench_plan_cache_session(
    group: &mut BenchmarkGroup<'_, WallTime>,
    registry: &BuiltinProcedureRegistry,
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    mut session: selene_gql::Session<'_>,
) {
    group.bench_function(
        BenchmarkId::new(
            "shared_session_plan_cache_query_root_text_score",
            row_label(fixture, model_id, corpus_label, quality),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(
                    fixture.execute_all_text_score_queries_in_session(&mut session, registry),
                );
            });
        },
    );
}

fn row_label(
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
) -> String {
    row_label_with_candidate_count(
        fixture,
        model_id,
        corpus_label,
        quality,
        fixture.first_query_expanded_count(),
    )
}

fn row_label_with_candidate_count(
    fixture: &OmlxGqlQueryRootFixture,
    model_id: &str,
    corpus_label: &str,
    quality: TextScoreQuality,
    candidate_count: usize,
) -> String {
    let mut label = format!(
        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}_curbp{}",
        model_id,
        corpus_label,
        fixture.query_count(),
        TOP_K,
        fixture.first_query_root_count(),
        candidate_count,
        fixture.dimension,
        quality.precision,
        quality.current_precision,
    );
    if let Some(target_hit) = quality.target_hit {
        label.push_str(&format!("_hitbp{target_hit}"));
    }
    label
}
