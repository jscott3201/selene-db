#![allow(missing_docs)]
//! Gating bench for the real graph→snapshot rkyv path (D14).
//!
//! `selene-persist`'s `snapshot` bench measures the SLSN *container* (framing +
//! compression + body hash) over synthetic opaque byte payloads. It never
//! touches the actual `CoreProvider` encode of real node/edge rows. This bench
//! drives that path end to end through the public provider surface:
//!
//! - `encode`    — `IndexProvider::write_section` over every `CORE/*` sub-tag
//!   (the rkyv archive of `CORE/NODE` + `CORE/EDGE` positional rows, D14).
//! - `decode`    — feed those section bytes back into a recovery-mode provider
//!   and `finish_recovery` (the positional placement / id↔row rebuild).
//! - `roundtrip` — encode then decode in one sample (the full D14 cost).
//!
//! Any CORE-06 `Value` reshape or D22 id/row remap changes these numbers; the
//! synthetic-bytes persist bench cannot see it.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::hint::black_box;
use std::sync::Arc;

use arc_swap::ArcSwap;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_graph::{CoreProvider, IndexProvider, SeleneGraph, SubTag};

/// Build a live-mode `CoreProvider` pointing at `graph`, plus the ordered list
/// of `CORE/*` sub-tags it declares. The `ArcSwap` mirrors how `SharedGraph`
/// wires the snapshot pointer the writer encodes from.
fn live_provider(graph: &SeleneGraph) -> (Arc<CoreProvider>, Vec<SubTag>) {
    let snapshot = Arc::new(ArcSwap::from_pointee(graph.clone()));
    let provider = CoreProvider::new_for_live(snapshot);
    let tags = provider.declared_sub_tags().to_vec();
    (provider, tags)
}

/// Encode every declared section, returning `(sub_tag, bytes)` pairs in the
/// canonical snapshot order.
fn encode_all(provider: &CoreProvider, tags: &[SubTag]) -> Vec<(SubTag, Vec<u8>)> {
    tags.iter()
        .map(|&sub| {
            let bytes = IndexProvider::write_section(provider, sub).expect("section encodes");
            (sub, bytes)
        })
        .collect()
}

/// Drain encoded sections into a fresh recovery-mode provider and materialize
/// the graph (the positional D14 decode).
fn decode_all(
    sections: &[(SubTag, Vec<u8>)],
    graph_id: selene_core::GraphId,
    bound_type: Option<Arc<selene_graph::graph_types::GraphTypeDef>>,
) -> SeleneGraph {
    let provider = CoreProvider::new_for_recovery();
    for (sub, bytes) in sections {
        IndexProvider::read_section(provider.as_ref(), *sub, bytes).expect("section decodes");
    }
    provider
        .finish_recovery(graph_id, bound_type)
        .expect("recovery materializes graph")
}

fn elements(graph: &SeleneGraph) -> u64 {
    (graph.node_count() + graph.edge_count()) as u64
}

/// Decode `sections` once and assert the materialized graph matches `source`'s
/// node/edge counts — a one-shot correctness guard so the bench panics on a
/// broken roundtrip instead of silently timing a degenerate decode.
fn assert_roundtrip_faithful(
    source: &SeleneGraph,
    sections: &[(SubTag, Vec<u8>)],
    graph_id: selene_core::GraphId,
    bound_type: &Option<Arc<selene_graph::graph_types::GraphTypeDef>>,
) {
    let restored = decode_all(sections, graph_id, bound_type.clone());
    assert_eq!(
        restored.node_count(),
        source.node_count(),
        "snapshot roundtrip must preserve node count"
    );
    assert_eq!(
        restored.edge_count(),
        source.edge_count(),
        "snapshot roundtrip must preserve edge count"
    );
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_snapshot_roundtrip/encode");
    for fixture in common::fixtures() {
        let graph = fixture.graph();
        let (provider, tags) = live_provider(graph);
        group.throughput(Throughput::Elements(elements(graph)));
        group.bench_function(BenchmarkId::from_parameter(graph.node_count()), |b| {
            b.iter(|| {
                let mut total = 0usize;
                for &sub in &tags {
                    total += IndexProvider::write_section(provider.as_ref(), sub)
                        .expect("section encodes")
                        .len();
                }
                black_box(total)
            });
        });
    }
    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_snapshot_roundtrip/decode");
    for fixture in common::fixtures() {
        let graph = fixture.graph();
        let graph_id = graph.graph_id();
        let bound_type = graph.meta.bound_type.clone();
        let (provider, tags) = live_provider(graph);
        let sections = encode_all(provider.as_ref(), &tags);
        // Self-validate once (untimed): a silent encode/decode break that
        // returns an empty or wrong graph would otherwise "pass" while
        // measuring nothing. Assert the roundtrip is faithful before timing.
        assert_roundtrip_faithful(graph, &sections, graph_id, &bound_type);
        group.throughput(Throughput::Elements(elements(graph)));
        group.bench_function(BenchmarkId::from_parameter(graph.node_count()), |b| {
            b.iter(|| {
                let restored = decode_all(&sections, graph_id, bound_type.clone());
                black_box(restored.node_count())
            });
        });
    }
    group.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_snapshot_roundtrip/roundtrip");
    for fixture in common::fixtures() {
        let graph = fixture.graph();
        let graph_id = graph.graph_id();
        let bound_type = graph.meta.bound_type.clone();
        let (provider, tags) = live_provider(graph);
        assert_roundtrip_faithful(
            graph,
            &encode_all(provider.as_ref(), &tags),
            graph_id,
            &bound_type,
        );
        group.throughput(Throughput::Elements(elements(graph)));
        group.bench_function(BenchmarkId::from_parameter(graph.node_count()), |b| {
            b.iter_batched(
                || (),
                |()| {
                    let sections = encode_all(provider.as_ref(), &tags);
                    let restored = decode_all(&sections, graph_id, bound_type.clone());
                    black_box(restored.node_count())
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = graph_snapshot_roundtrip;
    config = common::criterion_config();
    targets = bench_encode, bench_decode, bench_roundtrip
}
criterion_main!(graph_snapshot_roundtrip);
