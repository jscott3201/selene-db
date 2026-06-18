#![allow(missing_docs)]
//! GQLRT-05 gating bench: correlated `EXISTS` / aggregate `VALUE` subquery execution.
//!
//! A correlated subquery is re-evaluated for every outer row, and the schema
//! for its pattern is rebuilt per outer row (`schema_for_pattern`). This is the
//! only read-query EXECUTION bench in the suite — `expression_eval` is
//! scalar-only and `write_e2e` is write-only — and no `EXISTS`/aggregate `VALUE`
//! subquery exists in any other corpus, so a memoization win (GQLRT-05) would
//! otherwise be invisible. Runs on an IN-MEMORY graph (no WAL) so the per-row
//! schema rebuild dominates the sample, not durability.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_gql::Session;
use selene_graph::SharedGraph;
use selene_testing::{BenchFixture, BenchProfile};

/// Correlated existential subquery: re-evaluated per outer `Person` row.
const EXISTS_Q: &str = "MATCH (p:Person) FILTER EXISTS { MATCH (p)-[:KNOWS]->(:Person) } RETURN p";
/// Correlated aggregate value subquery in the projection: also per outer row.
const COUNT_Q: &str = "MATCH (p:Person) RETURN p, VALUE { MATCH (p)-[:KNOWS]->(:Person) RETURN count(*) } AS knows_people";

/// A *small* scale envelope, independent of the shared node-scale profile.
/// Correlated re-evaluation is O(outer-rows × subquery), so the per-row schema
/// rebuild this bench targets (GQLRT-05) is fully visible at a few thousand
/// rows; the shared 10k/50k/100k envelope makes a single arm a 16-minute
/// fsync-free marathon (10k already costs ~1.3 s/iter). These three points show
/// the linear-in-rows trend while keeping every arm well under a second-class
/// budget.
fn correlated_scales() -> &'static [usize] {
    match BenchProfile::from_env() {
        BenchProfile::Quick => &[1_000],
        BenchProfile::Full | BenchProfile::Stress => &[2_500, 5_000, 10_000],
        _ => &[1_000],
    }
}

fn bench_correlated(c: &mut Criterion) {
    let mut group = c.benchmark_group("gql_correlated_subquery");
    for &scale in correlated_scales() {
        // Fixture build + plan happen outside the timed routine; the in-memory
        // graph carries ~scale/3 Person rows, each driving one subquery eval.
        let fixture = BenchFixture::build(scale);
        let graph = SharedGraph::from_graph(fixture.graph().clone());
        for (name, source) in [("exists", EXISTS_Q), ("count", COUNT_Q)] {
            let plan = common::plan_write(source);
            group.throughput(Throughput::Elements(scale as u64));
            group.bench_function(BenchmarkId::new(name, scale), |b| {
                b.iter(|| {
                    let mut session = Session::new(&graph);
                    let rows = common::execute_preplanned(&plan, &mut session);
                    std::hint::black_box(rows)
                });
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = gql_correlated_subquery;
    config = common::criterion_config();
    targets = bench_correlated
}
criterion_main!(gql_correlated_subquery);
