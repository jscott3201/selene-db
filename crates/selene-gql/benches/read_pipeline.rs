#![allow(missing_docs)]
//! Read-query pipeline execution bench: closes the read-execution coverage
//! gap (the declared 60%-read workload previously had only the correlated
//! subquery bench timing read execution).
//!
//! Six warm-plan-cache rows run on a no-WAL in-memory `SharedGraph`, so the
//! timed body is pure execution + index access — not parse/plan/optimize and
//! not durability: label scan + indexed range filter, two-leg hash join,
//! ORDER BY top-K, high-cardinality GROUP BY, DISTINCT dedup, and bare
//! `LIMIT 10` (the scan short-circuit signal row). One `/cold` companion on
//! the cheapest row re-plans per iteration with an uncached session,
//! capturing the full read compile+execute pipeline.
//!
//! Fixture topology note: every `KNOWS` offset in `BenchFixture` is ≡1 mod 3,
//! so Person edges land on Sensor and Sensor edges land on Device —
//! Person→Person is ~empty. The join row targets Sensor/Device deliberately;
//! restricting the far legs to `:Person` would bench an empty join.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::num::NonZeroUsize;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_gql::{EmptyProcedureRegistry, Session, StatementOutput};
use selene_testing::BenchProfile;

/// Label scan + indexed `age` range filter + projection (~half of Persons).
const FILTER_PROJECT_Q: &str = "MATCH (n:Person) FILTER n.age >= 40 RETURN n.name AS name";
/// Two comma-separated legs sharing `s` lower to a hash join (build/probe).
const EXPAND_HASHJOIN_Q: &str = "MATCH (a:Person)-[:KNOWS]->(s:Sensor), \
     (s)-[:KNOWS]->(d:Device) RETURN a.name AS a_name, d.name AS d_name";
/// Full Person scan feeding a top-K sort; `score` is non-indexed.
const ORDER_BY_TOPK_Q: &str =
    "MATCH (n:Person) RETURN n.name AS name, n.score AS score ORDER BY n.score DESC LIMIT 10";
/// Hash-aggregate build over up to 1024 `score` groups.
const GROUP_BY_HIGHCARD_Q: &str =
    "MATCH (n:Person) RETURN n.score AS score, count(*) AS c GROUP BY n.score";
/// DISTINCT over 256 distinct `name` values (high dedup ratio).
const DISTINCT_DEDUP_Q: &str = "MATCH (n:Person) RETURN DISTINCT n.name AS name";
/// Bare LIMIT with no ORDER BY — the B19 scan short-circuit signal row.
const MATCH_LIMIT10_Q: &str = "MATCH (n:Person) RETURN n.name AS name LIMIT 10";

const WARM_ROWS: [(&str, &str); 6] = [
    ("match_filter_project", FILTER_PROJECT_Q),
    ("match_expand_hashjoin", EXPAND_HASHJOIN_Q),
    ("order_by_topk", ORDER_BY_TOPK_Q),
    ("group_by_highcard", GROUP_BY_HIGHCARD_Q),
    ("distinct_dedup", DISTINCT_DEDUP_Q),
    ("match_limit10", MATCH_LIMIT10_Q),
];

fn execute_read(session: &mut Session<'_>, source: &str) -> usize {
    match session
        .execute_source(source, &EmptyProcedureRegistry)
        .expect("read bench statement executes")
    {
        StatementOutput::Rows(table) => table.row_count(),
        other => panic!("read bench expects rows, got {other:?}"),
    }
}

fn bench_read_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_pipeline");
    for &scale in BenchProfile::from_env().scales() {
        // Fixture build happens once per scale, outside every timed routine.
        let state = common::gql_write_state_in_memory(scale);
        for (name, source) in WARM_ROWS {
            let mut session = Session::new(&state.graph)
                .with_plan_cache(NonZeroUsize::new(64).expect("nonzero"));
            // Seat the plan cache so the timed body is a pure cache-hit
            // execute (no parse/analyze/plan/optimize). Also asserts the row
            // is non-degenerate where the fixture guarantees output.
            let primed = execute_read(&mut session, source);
            if matches!(
                name,
                "match_expand_hashjoin" | "group_by_highcard" | "distinct_dedup"
            ) {
                assert!(primed > 0, "{name} produced no rows — fixture topology mismatch");
            }
            group.throughput(Throughput::Elements(scale as u64));
            group.bench_function(BenchmarkId::new(name, scale), |b| {
                b.iter(|| std::hint::black_box(execute_read(&mut session, source)));
            });
        }
        // Cold companion: a fresh cache-less session per iteration pays the
        // full parse/analyze/plan/optimize/execute pipeline. The graph is
        // shared (reads never mutate it); only the session is rebuilt.
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_function(BenchmarkId::new("match_limit10/cold", scale), |b| {
            b.iter_batched(
                || Session::new(&state.graph),
                |mut session| std::hint::black_box(execute_read(&mut session, MATCH_LIMIT10_Q)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = read_pipeline;
    config = common::criterion_config();
    targets = bench_read_pipeline
}
criterion_main!(read_pipeline);
