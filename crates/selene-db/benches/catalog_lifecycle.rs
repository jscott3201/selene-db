#![allow(missing_docs)]
//! Resolve, list, lifecycle publication, and named-graph selection costs.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_db::{
    CreatePolicy, Database, DropPolicy, ExecutionOutcome, GqlStatus, ObjectPath, SchemaPath,
};

const OMITTED: ExecutionOutcome = ExecutionOutcome::OmittedResult {
    status: GqlStatus::SUCCESSFUL_COMPLETION_OMITTED_RESULT,
};

fn schema(name: impl AsRef<str>) -> SchemaPath {
    SchemaPath::regular("selene", name.as_ref()).expect("benchmark schema path is valid")
}

fn graph(schema: &str, name: impl AsRef<str>) -> ObjectPath {
    ObjectPath::regular("selene", schema, name.as_ref()).expect("benchmark graph path is valid")
}

fn catalog_fixture(schema_count: usize) -> Database {
    let database = Database::builder().build();
    let catalog = database.catalog();
    for index in 0..schema_count {
        catalog
            .create_schema(&schema(format!("schema_{index:05}")), CreatePolicy::Strict)
            .expect("benchmark schema creation succeeds");
    }
    database
}

fn graph_fixture(graph_count: usize) -> Database {
    let database = Database::builder().build();
    let catalog = database.catalog();
    catalog
        .create_schema(&schema("graphs"), CreatePolicy::Strict)
        .expect("benchmark graph schema creation succeeds");
    for index in 0..graph_count {
        catalog
            .create_graph(
                &graph("graphs", format!("graph_{index:05}")),
                None,
                CreatePolicy::Strict,
            )
            .expect("benchmark graph creation succeeds");
    }
    database
}

fn catalog_scales() -> &'static [usize] {
    match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => &[16, 256, 1_024],
        _ => &[16, 256],
    }
}

fn graph_scales() -> &'static [usize] {
    match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => &[4, 32, 128],
        _ => &[4, 32],
    }
}

fn criterion_config() -> Criterion {
    let (samples, measurement_ms) = match std::env::var("SELENE_BENCH_PROFILE").ok().as_deref() {
        Some("full") | Some("stress") => (30, 1_500),
        _ => (10, 500),
    };
    Criterion::default()
        .sample_size(samples)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(measurement_ms))
}

fn bench_catalog_lifecycle(c: &mut Criterion) {
    let mut reads = c.benchmark_group("catalog_lifecycle/read");
    for &scale in catalog_scales() {
        let database = catalog_fixture(scale);
        let catalog = database.catalog();
        let target = schema(format!("schema_{:05}", scale / 2));
        reads.throughput(Throughput::Elements(1));
        reads.bench_with_input(BenchmarkId::new("resolve_schema", scale), &scale, |b, _| {
            b.iter(|| {
                black_box(
                    black_box(&catalog)
                        .snapshot()
                        .resolve_schema(black_box(&target))
                        .expect("benchmark schema exists"),
                )
            });
        });
        reads.bench_with_input(BenchmarkId::new("list_schemas", scale), &scale, |b, _| {
            b.iter(|| {
                black_box(
                    black_box(&catalog)
                        .snapshot()
                        .schemas()
                        .expect("benchmark catalog is valid"),
                )
            });
        });
        reads.bench_with_input(
            BenchmarkId::new("clone_outer_snapshot", scale),
            &scale,
            |b, _| {
                let snapshot = catalog.snapshot();
                b.iter(|| black_box(black_box(&snapshot).clone()));
            },
        );
    }
    reads.finish();

    let mut publication = c.benchmark_group("catalog_lifecycle/outer_snapshot_publication");
    for &scale in catalog_scales() {
        let database = catalog_fixture(scale);
        let catalog = database.catalog();
        let target = schema("publication_target");
        publication.bench_with_input(BenchmarkId::new("create_schema", scale), &scale, |b, _| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let started = std::time::Instant::now();
                    black_box(
                        catalog
                            .create_schema(&target, CreatePolicy::Strict)
                            .expect("timed schema create succeeds"),
                    );
                    elapsed += started.elapsed();
                    catalog
                        .drop_schema(&target, DropPolicy::Strict)
                        .expect("untimed schema cleanup succeeds");
                }
                elapsed
            });
        });
        catalog
            .create_schema(&target, CreatePolicy::Strict)
            .expect("drop fixture schema creation succeeds");
        publication.bench_with_input(BenchmarkId::new("drop_schema", scale), &scale, |b, _| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let started = std::time::Instant::now();
                    black_box(
                        catalog
                            .drop_schema(&target, DropPolicy::Strict)
                            .expect("timed schema drop succeeds"),
                    );
                    elapsed += started.elapsed();
                    catalog
                        .create_schema(&target, CreatePolicy::Strict)
                        .expect("untimed schema recreation succeeds");
                }
                elapsed
            });
        });
    }
    publication.finish();

    // GQL database-catalog DDL through a selected fixture graph at the same
    // schema scales as the Rust-API publication rows.
    let mut gql = c.benchmark_group("catalog_lifecycle/gql_ddl");
    for &scale in catalog_scales() {
        let database = catalog_fixture(scale);
        let catalog = database.catalog();
        let session_path = graph("schema_00000", "catalog_session");
        catalog
            .create_graph(&session_path, None, CreatePolicy::Strict)
            .expect("selected-session fixture graph creation succeeds");
        let session = database
            .session(&session_path)
            .expect("selected-session fixture resolves");
        let schema_target = schema("gql_target");
        let graph_target = graph("gql_target", "g");
        let graph_type_target = graph("gql_target", "shape");
        let bound_graph_target = graph("gql_target", "typed_g");
        gql.bench_with_input(
            BenchmarkId::new("create_schema_gql", scale),
            &scale,
            |b, _| {
                b.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let started = std::time::Instant::now();
                        let outcome = session
                            .execute("CREATE SCHEMA /gql_target")
                            .expect("timed GQL schema create succeeds");
                        elapsed += started.elapsed();
                        assert_eq!(black_box(outcome), OMITTED);
                        catalog
                            .drop_schema(&schema_target, DropPolicy::Strict)
                            .expect("untimed schema cleanup succeeds");
                    }
                    elapsed
                });
            },
        );
        catalog
            .create_schema(&schema_target, CreatePolicy::Strict)
            .expect("graph fixture schema creation succeeds");
        gql.bench_with_input(
            BenchmarkId::new("create_graph_type_gql", scale),
            &scale,
            |b, _| {
                b.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let started = std::time::Instant::now();
                        let outcome = session
                            .execute("CREATE GRAPH TYPE /gql_target/shape { NODE TYPE Person () }")
                            .expect("timed GQL graph-type create succeeds");
                        elapsed += started.elapsed();
                        assert_eq!(black_box(outcome), OMITTED);
                        catalog
                            .drop_graph_type(&graph_type_target, DropPolicy::Strict)
                            .expect("untimed graph-type cleanup succeeds");
                    }
                    elapsed
                });
            },
        );
        session
            .execute("CREATE GRAPH TYPE /gql_target/shape { NODE TYPE Person () }")
            .expect("bound-graph fixture type creation succeeds");
        gql.bench_with_input(
            BenchmarkId::new("create_graph_gql", scale),
            &scale,
            |b, _| {
                b.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let started = std::time::Instant::now();
                        let outcome = session
                            .execute("CREATE GRAPH /gql_target/g ANY")
                            .expect("timed GQL graph create succeeds");
                        elapsed += started.elapsed();
                        assert_eq!(black_box(outcome), OMITTED);
                        catalog
                            .drop_graph(&graph_target, DropPolicy::Strict)
                            .expect("untimed graph cleanup succeeds");
                    }
                    elapsed
                });
            },
        );
        gql.bench_with_input(
            BenchmarkId::new("create_bound_graph_gql", scale),
            &scale,
            |b, _| {
                b.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let started = std::time::Instant::now();
                        let outcome = session
                            .execute("CREATE GRAPH /gql_target/typed_g TYPED /gql_target/shape")
                            .expect("timed GQL bound graph create succeeds");
                        elapsed += started.elapsed();
                        assert_eq!(black_box(outcome), OMITTED);
                        catalog
                            .drop_graph(&bound_graph_target, DropPolicy::Strict)
                            .expect("untimed bound-graph cleanup succeeds");
                    }
                    elapsed
                });
            },
        );
        catalog
            .create_graph(&graph_target, None, CreatePolicy::Strict)
            .expect("drop fixture graph creation succeeds");
        gql.bench_with_input(BenchmarkId::new("drop_graph_gql", scale), &scale, |b, _| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let started = std::time::Instant::now();
                    let outcome = session
                        .execute("DROP GRAPH /gql_target/g")
                        .expect("timed GQL graph drop succeeds");
                    elapsed += started.elapsed();
                    assert_eq!(black_box(outcome), OMITTED);
                    catalog
                        .create_graph(&graph_target, None, CreatePolicy::Strict)
                        .expect("untimed graph recreation succeeds");
                }
                elapsed
            });
        });
        // The fixture graph exists after the drop row's untimed recreation;
        // each replacement leaves exactly one graph at the path, so no
        // cleanup is needed between iterations.
        gql.bench_with_input(
            BenchmarkId::new("create_or_replace_graph_gql", scale),
            &scale,
            |b, _| {
                b.iter(|| {
                    assert_eq!(
                        black_box(&session)
                            .execute(black_box("CREATE OR REPLACE GRAPH /gql_target/g ANY"))
                            .expect("timed GQL graph replace succeeds"),
                        OMITTED
                    );
                });
            },
        );
        gql.bench_with_input(
            BenchmarkId::new("create_graph_if_not_exists_noop_gql", scale),
            &scale,
            |b, _| {
                b.iter(|| {
                    assert_eq!(
                        black_box(&session)
                            .execute(black_box("CREATE GRAPH IF NOT EXISTS /gql_target/g ANY"))
                            .expect("conditional no-op succeeds"),
                        OMITTED
                    );
                });
            },
        );
    }
    gql.finish();

    let mut selection = c.benchmark_group("catalog_lifecycle/session_creation");
    for &scale in graph_scales() {
        let database = graph_fixture(scale);
        let target = graph("graphs", format!("graph_{:05}", scale / 2));
        selection.bench_with_input(BenchmarkId::new("resolve_graph", scale), &scale, |b, _| {
            b.iter(|| {
                black_box(
                    black_box(&database)
                        .session(black_box(&target))
                        .expect("benchmark graph exists"),
                )
            });
        });
    }
    selection.finish();
}

criterion_group! {
    name = catalog_lifecycle;
    config = criterion_config();
    targets = bench_catalog_lifecycle
}
criterion_main!(catalog_lifecycle);
