//! Opt-in construction plus one-row-query timing comparison.

use std::{hint::black_box, time::Instant};

use selene_core::GraphId;
use selene_db::{CreatePolicy, Database, ObjectPath, SchemaPath};
use selene_gql::{BuiltinProcedureRegistry, Session as LowerSession};
use selene_graph::SharedGraph;

const WARMUP_ITERATIONS: usize = 100;
const MEASURED_ITERATIONS: usize = 1_000;
const QUERY: &str = "RETURN 1";

#[test]
#[ignore = "opt-in local timing evidence; no performance threshold"]
#[allow(
    clippy::print_stdout,
    reason = "the opt-in measurement reports its samples"
)]
fn compare_facade_with_direct_lower_session() {
    let direct_graph = SharedGraph::new(GraphId::new(77));
    let direct_registry = BuiltinProcedureRegistry::new();
    let facade_database = Database::builder().build();
    let schema = SchemaPath::regular("selene", "bench").unwrap();
    let graph = ObjectPath::regular("selene", "bench", "main").unwrap();
    let catalog = facade_database.catalog();
    catalog
        .create_schema(&schema, CreatePolicy::Strict)
        .unwrap();
    catalog
        .create_graph(&graph, None, CreatePolicy::Strict)
        .unwrap();

    for _ in 0..WARMUP_ITERATIONS {
        run_direct(&direct_graph, &direct_registry);
        run_facade(&facade_database, &graph);
    }

    let direct_started = Instant::now();
    for _ in 0..MEASURED_ITERATIONS {
        run_direct(&direct_graph, &direct_registry);
    }
    let direct = direct_started.elapsed();

    let facade_started = Instant::now();
    for _ in 0..MEASURED_ITERATIONS {
        run_facade(&facade_database, &graph);
    }
    let facade = facade_started.elapsed();

    println!(
        "facade_overhead warmup={} iterations={} direct_total_ns={} direct_mean_ns={} facade_total_ns={} facade_mean_ns={}",
        WARMUP_ITERATIONS,
        MEASURED_ITERATIONS,
        direct.as_nanos(),
        direct.as_nanos() / MEASURED_ITERATIONS as u128,
        facade.as_nanos(),
        facade.as_nanos() / MEASURED_ITERATIONS as u128,
    );
}

fn run_direct(graph: &SharedGraph, registry: &BuiltinProcedureRegistry) {
    let mut session = LowerSession::new(graph);
    black_box(
        session
            .execute_source(black_box(QUERY), registry)
            .expect("direct query succeeds"),
    );
}

fn run_facade(database: &Database, graph: &ObjectPath) {
    let session = database.session(graph).unwrap();
    black_box(
        session
            .execute(black_box(QUERY))
            .expect("facade query succeeds"),
    );
}
