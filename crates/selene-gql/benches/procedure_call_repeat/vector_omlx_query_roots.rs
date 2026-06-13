use std::{hint::black_box, num::NonZeroUsize, sync::Arc};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_gql::{BuiltinProcedureRegistry, CallPlanCache};
use selene_testing::local_omlx::EmbeddingBenchConfig;

#[path = "vector_omlx_query_roots/fixture.rs"]
mod fixture;
#[path = "vector_omlx_query_roots/json_score_rows.rs"]
mod json_score_rows;
#[path = "vector_omlx_query_roots/labels.rs"]
mod labels;
#[path = "vector_omlx_query_roots/state_batch_rows.rs"]
mod state_batch_rows;
#[path = "vector_omlx_query_roots/text_score_rows.rs"]
mod text_score_rows;

use fixture::{OmlxGqlQueryRootFixture, TOP_K};
use labels::{append_target_hit, corpus_label, model_id};

pub(super) fn bench_vector_omlx_query_roots_procedure(c: &mut Criterion) {
    let Some(config) = EmbeddingBenchConfig::from_env() else {
        return;
    };
    let inputs = config.inputs();
    let registry = BuiltinProcedureRegistry::new();
    let mut group = c.benchmark_group("procedure_vector_omlx_query_roots");
    for model in &config.models {
        let model_id = model_id(model);
        let vectors = config
            .embed(model, &inputs)
            .expect("embedding request succeeds");
        let fixture = OmlxGqlQueryRootFixture::build(
            model,
            &inputs,
            vectors,
            config.graph_hint_docs_per_topic,
        );
        let cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let anchor_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let anchor_batch_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let root_rows_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let root_rows_batch_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let root_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let root_batch_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let state_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let current_state_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let provenance_state_cache =
            Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        let batch_cache = Arc::new(CallPlanCache::new(NonZeroUsize::new(256).expect("nonzero")));
        fixture.warm_query_anchor_lookup_cache(&registry, Arc::clone(&anchor_cache));
        fixture.warm_query_anchor_lookup_batch_cache(&registry, Arc::clone(&anchor_batch_cache));
        fixture.warm_query_root_rows_cache(&registry, Arc::clone(&root_rows_cache));
        fixture.warm_query_root_rows_batch_cache(&registry, Arc::clone(&root_rows_batch_cache));
        fixture.warm_query_root_materialize_cache(&registry, Arc::clone(&root_cache));
        fixture.warm_query_root_materialize_batch_cache(&registry, Arc::clone(&root_batch_cache));
        fixture.warm_query_root_cache(&registry, Arc::clone(&cache));
        fixture.warm_query_root_state_cache(&registry, Arc::clone(&state_cache));
        fixture.warm_query_root_current_state_cache(&registry, Arc::clone(&current_state_cache));
        fixture
            .warm_query_root_provenance_state_cache(&registry, Arc::clone(&provenance_state_cache));
        fixture.warm_query_root_batch_cache(&registry, Arc::clone(&batch_cache));
        let mut anchor_session = fixture.reusable_session();
        let mut anchor_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_anchor_lookup_session(&mut anchor_plan_session, &registry);
        let mut root_materialize_session = fixture.reusable_session();
        let mut root_materialize_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_root_materialize_session(&mut root_materialize_plan_session, &registry);
        let mut expansion_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_query_root_expansion_session(&mut expansion_plan_session, &registry);
        let mut state_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_query_root_state_session(&mut state_plan_session, &registry);
        let mut current_state_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_query_root_current_state_session(&mut current_state_plan_session, &registry);
        let mut provenance_state_plan_session = fixture.reusable_plan_cache_session();
        fixture.warm_query_root_provenance_state_session(
            &mut provenance_state_plan_session,
            &registry,
        );
        let precision = fixture.gql_precision_basis_points(&registry, Some(Arc::clone(&cache)));
        let current_precision =
            fixture.gql_current_precision_basis_points(&registry, Some(Arc::clone(&cache)));
        let state_precision =
            fixture.gql_state_precision_basis_points(&registry, Some(Arc::clone(&state_cache)));
        let current_state_precision = fixture.gql_current_state_precision_basis_points(
            &registry,
            Some(Arc::clone(&current_state_cache)),
        );
        let provenance_state_precision = fixture
            .gql_provenance_state_current_precision_basis_points(
                &registry,
                Some(Arc::clone(&provenance_state_cache)),
            );
        let batch_precision =
            fixture.gql_batch_precision_basis_points(&registry, Some(Arc::clone(&batch_cache)));
        let target_hit = fixture.gql_target_hit_basis_points(&registry, Some(Arc::clone(&cache)));
        let state_target_hit =
            fixture.gql_state_target_hit_basis_points(&registry, Some(Arc::clone(&state_cache)));
        let current_state_target_hit = fixture.gql_current_state_target_hit_basis_points(
            &registry,
            Some(Arc::clone(&current_state_cache)),
        );
        let provenance_state_target_hit = fixture.gql_provenance_state_target_hit_basis_points(
            &registry,
            Some(Arc::clone(&provenance_state_cache)),
        );
        let batch_target_hit =
            fixture.gql_batch_target_hit_basis_points(&registry, Some(Arc::clone(&batch_cache)));
        group.throughput(Throughput::Elements((fixture.query_count() * TOP_K) as u64));
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_anchor_lookup",
                format!(
                    "{}_{}_q{}_anchors{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.query_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_anchor_lookup_queries(
                        &registry,
                        Some(Arc::clone(&anchor_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_query_anchor_lookup",
                format!(
                    "{}_{}_q{}_anchors{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.query_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_anchor_lookup_queries_in_session(
                        &mut anchor_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_anchor_lookup",
                format!(
                    "{}_{}_q{}_anchors{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.query_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_anchor_lookup_queries_in_session(
                        &mut anchor_plan_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_anchor_lookup_batch",
                format!(
                    "{}_{}_q{}_anchors{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.query_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_anchor_lookup_batch_count(
                        &registry,
                        Some(Arc::clone(&anchor_batch_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_rows",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_root_rows_queries(
                        &registry,
                        Some(Arc::clone(&root_rows_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_rows_batch",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_root_rows_batch_count(
                        &registry,
                        Some(Arc::clone(&root_rows_batch_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_materialize",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_root_materialize_queries(
                        &registry,
                        Some(Arc::clone(&root_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_query_root_materialize",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_root_materialize_queries_in_session(
                        &mut root_materialize_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_root_materialize",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_root_materialize_queries_in_session(
                        &mut root_materialize_plan_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_materialize_batch",
                format!(
                    "{}_{}_q{}_r{}_totalr{}_dim{}",
                    model_id,
                    corpus_label(config.corpus),
                    fixture.query_count(),
                    fixture.first_query_root_count(),
                    fixture.query_count() * fixture.first_query_root_count(),
                    fixture.dimension,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_root_materialize_batch_count(
                        &registry,
                        Some(Arc::clone(&root_batch_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_expansion",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_expanded_count(),
                        fixture.dimension,
                        precision,
                    ),
                    target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_queries(&registry, Some(Arc::clone(&cache))));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_root_expansion",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_expanded_count(),
                        fixture.dimension,
                        precision,
                    ),
                    target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture
                            .execute_all_queries_in_session(&mut expansion_plan_session, &registry),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_state_intersection_count(),
                        fixture.dimension,
                        state_precision,
                    ),
                    state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture
                            .execute_all_state_queries(&registry, Some(Arc::clone(&state_cache))),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_root_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_state_intersection_count(),
                        fixture.dimension,
                        state_precision,
                    ),
                    state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture.execute_all_state_queries_in_session(
                            &mut state_plan_session,
                            &registry,
                        ),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_current_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_basecurbp{}_curbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_current_state_intersection_count(),
                        fixture.dimension,
                        current_precision,
                        current_state_precision,
                    ),
                    current_state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_current_state_queries(
                        &registry,
                        Some(Arc::clone(&current_state_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_root_current_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_basecurbp{}_curbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_current_state_intersection_count(),
                        fixture.dimension,
                        current_precision,
                        current_state_precision,
                    ),
                    current_state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_current_state_queries_in_session(
                        &mut current_state_plan_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_provenance_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_basecurbp{}_curbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_provenance_state_intersection_count(),
                        fixture.dimension,
                        current_precision,
                        provenance_state_precision,
                    ),
                    provenance_state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_provenance_state_queries(
                        &registry,
                        Some(Arc::clone(&provenance_state_cache)),
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_session_plan_cache_query_root_provenance_state_intersection",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_basecurbp{}_curbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_provenance_state_intersection_count(),
                        fixture.dimension,
                        current_precision,
                        provenance_state_precision,
                    ),
                    provenance_state_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(fixture.execute_all_provenance_state_queries_in_session(
                        &mut provenance_state_plan_session,
                        &registry,
                    ));
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                "shared_cache_query_root_expansion_batch",
                append_target_hit(
                    format!(
                        "{}_{}_q{}_k{}_r{}_c{}_dim{}_precbp{}",
                        model_id,
                        corpus_label(config.corpus),
                        fixture.query_count(),
                        TOP_K,
                        fixture.first_query_root_count(),
                        fixture.first_query_expanded_count(),
                        fixture.dimension,
                        batch_precision,
                    ),
                    batch_target_hit,
                ),
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        fixture.execute_batch_query(&registry, Some(Arc::clone(&batch_cache))),
                    );
                });
            },
        );
        state_batch_rows::bench_state_batch_rows(
            &mut group,
            &registry,
            &fixture,
            &model_id,
            corpus_label(config.corpus),
            current_precision,
        );
        text_score_rows::bench_text_score_rows(
            &mut group,
            &registry,
            &fixture,
            &model_id,
            corpus_label(config.corpus),
        );
        json_score_rows::bench_json_score_rows(
            &mut group,
            &registry,
            &fixture,
            &model_id,
            corpus_label(config.corpus),
        );
    }
    group.finish();
}
