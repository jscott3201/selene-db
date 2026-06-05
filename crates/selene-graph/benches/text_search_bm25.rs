#![allow(missing_docs)]
//! Criterion benches for exact BM25 text search.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_graph::{SeleneGraph, SharedGraph};
use selene_testing::BenchProfile;

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
    }
    group.finish();
}

struct TextFixture {
    graph: Arc<SeleneGraph>,
    label: IStr,
    property: IStr,
    query: String,
}

impl TextFixture {
    fn build(scale: usize) -> Self {
        let shared = SharedGraph::new(GraphId::new(431_201));
        let label = intern("BenchTextDoc").expect("bench label interns");
        let property = intern("body").expect("bench property interns");
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
                            Value::String(intern(&text).expect("bench text interns")),
                        ),
                    )
                    .expect("bench node inserts");
            }
            txn.commit().expect("bench fixture commits");
        }
        Self {
            graph: shared.read(),
            label,
            property,
            query: "gql current retrieval evidence".to_owned(),
        }
    }
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("bench property map is valid")
}

criterion_group! {
    name = text_search;
    config = common::criterion_config();
    targets = bench_exact_bm25
}
criterion_main!(text_search);
