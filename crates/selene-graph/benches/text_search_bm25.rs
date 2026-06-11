#![allow(missing_docs)]
//! Criterion benches for exact and indexed BM25 text search.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod text_search_bm25_hybrid;

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, DbString, GraphId, LabelSet, PropertyMap, Value, db_string,
};
use selene_graph::{SeleneGraph, SharedGraph, TextIndex};
use selene_testing::BenchProfile;
use text_search_bm25_hybrid::bench_hybrid_bm25_vector;

const TOPICS: [&str; 8] = [
    "gql",
    "vector",
    "memory",
    "planner",
    "storage",
    "agent",
    "schema",
    "retrieval",
];
const STATES: [&str; 4] = ["current", "stale", "draft", "verified"];

fn bench_exact_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_text_bm25_exact");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = TextFixture::build(scale);
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_with_input(
            BenchmarkId::new("topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph
                        .exact_text_search_nodes(
                            &fixture.label,
                            &fixture.property,
                            &fixture.query,
                            10,
                        )
                        .expect("BM25 search succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("topic_query_checked_with_deadline", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                let checker = deadline_checker();
                b.iter(|| {
                    let hits = fixture
                        .graph
                        .exact_text_search_nodes_checked(
                            &fixture.label,
                            &fixture.property,
                            &fixture.query,
                            10,
                            checker,
                        )
                        .expect("BM25 search succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

fn bench_indexed_bm25(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_text_bm25_indexed");
    for &scale in BenchProfile::from_env().scales() {
        let fixture = TextFixture::build(scale);
        group.throughput(Throughput::Elements(scale as u64));
        group.bench_with_input(
            BenchmarkId::new("prebuilt_topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture.index.search(&fixture.query, 10);
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("registered_topic_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture.registered_index.search(&fixture.query, 10);
                    std::hint::black_box(hits.len());
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("transient_build_query", format!("n{scale}_k10")),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let hits = fixture
                        .graph
                        .indexed_text_search_nodes(
                            &fixture.label,
                            &fixture.property,
                            &fixture.query,
                            10,
                        )
                        .expect("indexed BM25 search succeeds");
                    std::hint::black_box(hits.len());
                });
            },
        );
    }
    group.finish();
}

struct TextFixture {
    graph: Arc<SeleneGraph>,
    label: DbString,
    property: DbString,
    query: String,
    index: TextIndex,
    registered_index: Arc<TextIndex>,
}

impl TextFixture {
    fn build(scale: usize) -> Self {
        let shared = SharedGraph::new(GraphId::new(431_201));
        let label = db_string("BenchTextDoc").expect("bench label fits DB string cap");
        let property = db_string("body").expect("bench property fits DB string cap");
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for row in 0..scale {
                let topic = TOPICS[row % TOPICS.len()];
                let neighbor = TOPICS[(row + 3) % TOPICS.len()];
                let state = STATES[row % STATES.len()];
                let text = format!(
                    "{topic} {state} agent memory fact supports {neighbor} retrieval evidence"
                );
                mutator
                    .create_node(
                        LabelSet::single(label.clone()),
                        props(
                            &property,
                            Value::String(db_string(&text).expect("bench text fits DB string cap")),
                        ),
                    )
                    .expect("bench node inserts");
            }
            txn.commit().expect("bench fixture commits");
        }
        shared
            .create_text_index(label.clone(), property.clone())
            .expect("bench text index registers");
        let graph = shared.read();
        let index = graph
            .build_text_index(&label, &property)
            .expect("bench text index builds");
        let registered_index = graph
            .text_index_for(&label, &property)
            .expect("registered bench text index exists");
        Self {
            graph,
            label,
            property,
            query: "gql current retrieval evidence".to_owned(),
            index,
            registered_index,
        }
    }
}

fn deadline_checker() -> CancellationChecker<'static> {
    CancellationChecker::new(None, Some(Instant::now() + Duration::from_secs(3600)))
}

fn props(key: &DbString, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("bench property map is valid")
}

criterion_group! {
    name = text_search;
    config = common::criterion_config();
    targets = bench_exact_bm25, bench_indexed_bm25, bench_hybrid_bm25_vector
}
criterion_main!(text_search);
