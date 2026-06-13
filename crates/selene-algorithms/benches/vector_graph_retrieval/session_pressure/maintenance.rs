//! Active-set maintenance rows for graph/vector retrieval pressure benchmarks.

use std::collections::HashSet;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::NodeId;

use crate::common::scale_label;

use super::super::support::{FACTS_PER_TOPIC, RESULT_K, basis_points, vector_scales};
use super::super::{MemoryRetrievalFixture, TopologyNoise};
use super::SessionStrategy;

const READS_PER_CYCLE: usize = 60;
const WRITES_PER_CYCLE: usize = 40;
const WRITE_ROUNDS: usize = 10;
const READS_PER_WRITE_ROUND: usize = READS_PER_CYCLE / WRITE_ROUNDS;
const REMOVE_INSERT_PAIRS_PER_ROUND: usize = 2;

pub(super) fn bench_active_set_maintenance_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_active_set_maintenance_pressure");
    for scale in vector_scales() {
        let fixture = MemoryRetrievalFixture::build_with_topology(
            scale,
            TopologyNoise::ContradictedCurrentDuplicates,
        );
        let write_nodes = active_set_write_nodes(&fixture);
        bench_dynamic_read_write(&mut group, &fixture, &write_nodes);
        bench_materialized_read_write(&mut group, &fixture, &write_nodes);
        bench_materialized_write_only(&mut group, &fixture, &write_nodes);
    }
    group.finish();
}

fn bench_dynamic_read_write(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &MemoryRetrievalFixture,
    write_nodes: &[NodeId],
) {
    let strategy = SessionStrategy::GraphSessionUnresolvedCurrentFilter;
    let avg_candidates = fixture.average_session_candidates(strategy);
    let quality = fixture.session_quality(strategy);
    group.throughput(Throughput::Elements(
        (READS_PER_CYCLE * fixture.query_count() * avg_candidates + WRITES_PER_CYCLE) as u64,
    ));
    group.bench_function(
        BenchmarkId::new(
            "dynamic_edge_checks_r60w40",
            quality_id(fixture, avg_candidates, quality),
        ),
        |b| {
            b.iter(|| {
                black_box(dynamic_read_write_cycle(fixture, write_nodes));
            });
        },
    );
}

fn bench_materialized_read_write(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &MemoryRetrievalFixture,
    write_nodes: &[NodeId],
) {
    let strategy = SessionStrategy::GraphSessionMaterializedUnresolvedCurrentFilter;
    let avg_candidates = fixture.average_session_candidates(strategy);
    let quality = fixture.session_quality(strategy);
    let mut active_set = fixture.graph_unresolved_current_nodes.clone();
    group.throughput(Throughput::Elements(
        (READS_PER_CYCLE * fixture.query_count() * avg_candidates + WRITES_PER_CYCLE) as u64,
    ));
    group.bench_function(
        BenchmarkId::new(
            "materialized_set_r60w40",
            quality_id(fixture, avg_candidates, quality),
        ),
        |b| {
            b.iter(|| {
                black_box(materialized_read_write_cycle(
                    fixture,
                    &mut active_set,
                    write_nodes,
                ));
            });
        },
    );
}

fn bench_materialized_write_only(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    fixture: &MemoryRetrievalFixture,
    write_nodes: &[NodeId],
) {
    let mut active_set = fixture.graph_unresolved_current_nodes.clone();
    group.throughput(Throughput::Elements(WRITES_PER_CYCLE as u64));
    group.bench_function(
        BenchmarkId::new(
            "materialized_set_maintenance_w40",
            format!(
                "{}_active{}_w{}",
                scale_label(fixture.scale()),
                active_set.len(),
                WRITES_PER_CYCLE
            ),
        ),
        |b| {
            b.iter(|| {
                black_box(materialized_write_cycle(&mut active_set, write_nodes));
            });
        },
    );
}

fn quality_id(
    fixture: &MemoryRetrievalFixture,
    avg_candidates: usize,
    quality: super::super::RetrievalQuality,
) -> String {
    format!(
        "{}_q{}_c{}_r{}w{}_covbp{}_curbp{}_precbp{}",
        scale_label(fixture.scale()),
        fixture.query_count(),
        avg_candidates,
        READS_PER_CYCLE,
        WRITES_PER_CYCLE,
        basis_points(quality.coverage, fixture.query_count() * FACTS_PER_TOPIC),
        basis_points(
            quality.current_coverage,
            fixture.query_count() * FACTS_PER_TOPIC
        ),
        basis_points(quality.precision, fixture.query_count() * RESULT_K),
    )
}

fn dynamic_read_write_cycle(fixture: &MemoryRetrievalFixture, write_nodes: &[NodeId]) -> usize {
    let mut total = 0usize;
    for _ in 0..READS_PER_CYCLE {
        total = total.wrapping_add(
            fixture.session_total_coverage(SessionStrategy::GraphSessionUnresolvedCurrentFilter),
        );
    }
    for node in write_nodes.iter().cycle().take(WRITES_PER_CYCLE) {
        total ^= node.get() as usize;
    }
    total
}

fn materialized_read_write_cycle(
    fixture: &MemoryRetrievalFixture,
    active_set: &mut HashSet<NodeId>,
    write_nodes: &[NodeId],
) -> usize {
    let mut total = 0usize;
    for round in 0..WRITE_ROUNDS {
        for _ in 0..READS_PER_WRITE_ROUND {
            total = total.wrapping_add(fixture.session_total_coverage(
                SessionStrategy::GraphSessionMaterializedUnresolvedCurrentFilter,
            ));
        }
        total = total.wrapping_add(materialized_write_round(active_set, write_nodes, round));
    }
    debug_assert_eq!(
        active_set.len(),
        fixture.graph_unresolved_current_nodes.len()
    );
    total
}

fn materialized_write_cycle(active_set: &mut HashSet<NodeId>, write_nodes: &[NodeId]) -> usize {
    let mut total = 0usize;
    for round in 0..WRITE_ROUNDS {
        total = total.wrapping_add(materialized_write_round(active_set, write_nodes, round));
    }
    total
}

fn materialized_write_round(
    active_set: &mut HashSet<NodeId>,
    write_nodes: &[NodeId],
    round: usize,
) -> usize {
    let mut changed = 0usize;
    let offset = round * REMOVE_INSERT_PAIRS_PER_ROUND;
    for index in 0..REMOVE_INSERT_PAIRS_PER_ROUND {
        let node = write_nodes[(offset + index) % write_nodes.len()];
        changed += usize::from(active_set.remove(&node));
    }
    for index in 0..REMOVE_INSERT_PAIRS_PER_ROUND {
        let node = write_nodes[(offset + index) % write_nodes.len()];
        changed += usize::from(active_set.insert(node));
    }
    changed
}

fn active_set_write_nodes(fixture: &MemoryRetrievalFixture) -> Vec<NodeId> {
    let mut nodes: Vec<_> = fixture
        .graph_unresolved_current_nodes
        .iter()
        .copied()
        .collect();
    nodes.sort_unstable_by_key(|node| node.get());
    nodes.truncate(WRITE_ROUNDS * REMOVE_INSERT_PAIRS_PER_ROUND);
    nodes
}
