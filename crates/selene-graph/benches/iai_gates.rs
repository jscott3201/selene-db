#![allow(missing_docs)]
//! iai-callgrind gates for deterministic graph hot paths.

use iai_callgrind::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};
use selene_core::Value;
use selene_testing::BenchFixture;

#[library_benchmark]
fn node_fetch_1k() -> usize {
    let fixture = BenchFixture::build(1_000);
    fixture
        .graph()
        .node_properties(fixture.sample_node_id())
        .map(selene_core::PropertyMap::len)
        .unwrap_or_default()
}

#[library_benchmark]
fn label_index_1k() -> u64 {
    let fixture = BenchFixture::build(1_000);
    fixture
        .graph()
        .nodes_with_label(&fixture.sensor_label())
        .map(|rows| rows.len())
        .unwrap_or_default()
}

#[library_benchmark]
fn typed_index_point_1k() -> u64 {
    let fixture = BenchFixture::build(1_000);
    fixture
        .graph()
        .nodes_with_property_eq(
            &fixture.person_label(),
            &fixture.age_key(),
            &Value::Int(fixture.sample_age_value()),
        )
        .map(|rows| rows.len())
        .unwrap_or_default()
}

#[library_benchmark]
fn typed_index_range_1k() -> u64 {
    let fixture = BenchFixture::build(1_000);
    fixture
        .graph()
        .nodes_with_property_range(
            &fixture.person_label(),
            &fixture.age_key(),
            Value::Int(20)..Value::Int(60),
        )
        .map(|rows| rows.len())
        .unwrap_or_default()
}

#[library_benchmark]
fn mutation_commit_100() -> usize {
    let fixture = BenchFixture::build(1_000);
    let shared = selene_graph::SharedGraph::from_graph(fixture.graph().clone());
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for idx in 0..100 {
            let diff = selene_core::PropertyDiff::new([(fixture.score_key(), Value::Int(idx))], [])
                .expect("diff is valid");
            mutator
                .update_node(
                    selene_core::NodeId::new(idx as u64 + 1),
                    selene_core::LabelDiff::new([], []).unwrap(),
                    diff,
                )
                .expect("update succeeds");
        }
    }
    std::hint::black_box(txn.commit().expect("commit succeeds").changes.len())
}

library_benchmark_group!(
    name = graph_gates;
    benchmarks = node_fetch_1k, label_index_1k, typed_index_point_1k,
        typed_index_range_1k, mutation_commit_100
);

fn gate_config() -> LibraryBenchmarkConfig {
    let mut callgrind = Callgrind::default();
    callgrind.soft_limits([(EventKind::Ir, 5.0)]);
    callgrind.fail_fast(true);

    let mut config = LibraryBenchmarkConfig::default();
    config.tool(callgrind);
    config
}

main!(
    config = gate_config();
    library_benchmark_groups = graph_gates
);
