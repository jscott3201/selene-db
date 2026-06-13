#![allow(missing_docs)]
//! Criterion benches for delete-time payload clearing and row densification.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{DbString, GraphId, LabelSet, NodeId, PropertyMap, Value, VectorValue};
use selene_graph::{SeleneGraph, SharedGraph};
use selene_testing::BenchProfile;

const VECTOR_DIMENSION: usize = 768;
const DELETE_FRACTION_DENOMINATOR: usize = 10;

#[derive(Clone)]
struct VectorDeleteFixture {
    graph: SeleneGraph,
    targets: Vec<NodeId>,
    scale: usize,
    deleted_payload_kib: usize,
}

impl VectorDeleteFixture {
    fn build(scale: usize) -> Self {
        let scale = scale.max(1);
        let label = db_string("Embedding");
        let embedding = db_string("embedding");
        let bench_id = db_string("bench_id");
        let shared = SharedGraph::new(GraphId::new(1));
        let mut targets = Vec::with_capacity(delete_count(scale));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let props = PropertyMap::from_pairs([
                    (bench_id.clone(), Value::Int(idx as i64)),
                    (embedding.clone(), Value::Vector(vector_value(idx))),
                ])
                .expect("bench vector properties fit core caps");
                let id = mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node create succeeds");
                if targets.len() < delete_count(scale) {
                    targets.push(id);
                }
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        let deleted_payload_kib = targets.len() * VECTOR_DIMENSION * size_of::<f32>() / 1024;
        Self {
            graph: shared.read().as_ref().clone(),
            targets,
            scale,
            deleted_payload_kib,
        }
    }

    fn delete_count(&self) -> usize {
        self.targets.len()
    }

    fn live_after_delete(&self) -> usize {
        self.scale - self.delete_count()
    }

    fn id_suffix(&self) -> String {
        format!(
            "n{}_del{}_dim{}_payload{}k",
            compact_usize(self.scale),
            compact_usize(self.delete_count()),
            VECTOR_DIMENSION,
            compact_usize(self.deleted_payload_kib)
        )
    }
}

fn bench_vector_payload_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_delete_reclamation/vector_payload_delete");
    for scale in BenchProfile::from_env().scales() {
        let fixture = VectorDeleteFixture::build(*scale);
        group.throughput(Throughput::Elements(fixture.delete_count() as u64));
        group.bench_function(BenchmarkId::from_parameter(fixture.id_suffix()), |b| {
            b.iter_batched(
                || SharedGraph::from_graph(fixture.graph.clone()),
                |shared| {
                    let changes = delete_targets(&shared, &fixture.targets);
                    let snapshot = shared.read();
                    let dead_rows = snapshot.node_store.len() - snapshot.node_count();
                    assert_eq!(snapshot.node_count(), fixture.live_after_delete());
                    assert_eq!(snapshot.node_store.len(), fixture.scale);
                    assert_eq!(dead_rows, fixture.delete_count());
                    assert!(snapshot.node_properties(fixture.targets[0]).is_none());
                    drop(snapshot);
                    std::hint::black_box((shared, changes, dead_rows))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_compact_after_vector_delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_delete_reclamation/compact_after_vector_delete");
    for scale in BenchProfile::from_env().scales() {
        let fixture = VectorDeleteFixture::build(*scale);
        group.throughput(Throughput::Elements(fixture.delete_count() as u64));
        group.bench_function(BenchmarkId::from_parameter(fixture.id_suffix()), |b| {
            b.iter_batched(
                || {
                    let shared = SharedGraph::from_graph(fixture.graph.clone());
                    delete_targets(&shared, &fixture.targets);
                    shared
                },
                |shared| {
                    let before = shared.read();
                    assert_eq!(before.node_count(), fixture.live_after_delete());
                    assert_eq!(
                        before.node_store.len() - before.node_count(),
                        fixture.delete_count()
                    );
                    drop(before);
                    let report = shared.compact().expect("bench compaction succeeds");
                    assert_eq!(report.reclaimed_nodes as usize, fixture.delete_count());
                    assert_eq!(report.reclaimed_edges, 0);
                    let after = shared.read();
                    assert_eq!(after.node_store.len(), after.node_count());
                    assert_eq!(after.node_count(), fixture.live_after_delete());
                    drop(after);
                    std::hint::black_box((shared, report))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn delete_targets(shared: &SharedGraph, targets: &[NodeId]) -> usize {
    let mut txn = shared.begin_write();
    {
        let mut mutator = txn.mutator();
        for id in targets {
            mutator
                .delete_node(*id)
                .expect("bench node delete succeeds");
        }
    }
    txn.commit()
        .expect("bench delete commit succeeds")
        .changes
        .len()
}

fn delete_count(scale: usize) -> usize {
    (scale / DELETE_FRACTION_DENOMINATOR).max(1)
}

fn vector_value(seed: usize) -> VectorValue {
    let mut components = Vec::with_capacity(VECTOR_DIMENSION);
    for dim in 0..VECTOR_DIMENSION {
        let value = ((seed.wrapping_mul(31).wrapping_add(dim * 17)) % 1_000) as f32;
        components.push((value * 0.001) - 0.5);
    }
    VectorValue::new(components).expect("bench vector is finite and non-empty")
}

fn db_string(value: &str) -> DbString {
    selene_core::db_string(value).expect("bench string fits DB string cap")
}

fn compact_usize(value: usize) -> String {
    compact_count(u64::try_from(value).unwrap_or(u64::MAX))
}

fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{}m", value / 1_000_000)
    } else if value >= 1_000 {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

criterion_group! {
    name = graph_delete_reclamation;
    config = common::criterion_config();
    targets = bench_vector_payload_delete, bench_compact_after_vector_delete
}
criterion_main!(graph_delete_reclamation);
