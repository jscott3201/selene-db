#![allow(missing_docs)]
//! Mixed vector read/write workload benchmark.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap,
    Value, VectorMetric, VectorValue, intern,
};
use selene_graph::{ApproximateVectorSearchOptions, SharedGraph, VectorIndexKind};
use selene_testing::BenchProfile;

const DIMENSION: usize = 128;
const READS_PER_CYCLE: usize = 60;
const WRITES_PER_CYCLE: usize = 40;
const OPS_PER_CYCLE: usize = READS_PER_CYCLE + WRITES_PER_CYCLE;
const TOP_K: usize = 10;
const IVF_SEARCH_WIDTH: usize = 2;

fn bench_vector_mixed_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_mixed_workload");
    group.throughput(Throughput::Elements(OPS_PER_CYCLE as u64));
    for scale in vector_scales() {
        group.bench_with_input(
            BenchmarkId::new("ivf_cos_dim128_k10_r60w40_ef2", scale),
            &scale,
            |b, &scale| {
                b.iter_batched(
                    || MixedVectorFixture::build(scale),
                    |fixture| black_box(fixture.run_cycle()),
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

struct MixedVectorFixture {
    shared: SharedGraph,
    label: IStr,
    embedding_key: IStr,
    queries: Vec<VectorValue>,
    update_ids: Vec<NodeId>,
    update_vectors: Vec<VectorValue>,
}

impl MixedVectorFixture {
    fn build(scale: usize) -> Self {
        let scale = scale.max(WRITES_PER_CYCLE);
        let label = intern("VectorDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(12_000 + scale as u64));
        let mut update_ids = Vec::with_capacity(WRITES_PER_CYCLE);
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let vector = Value::Vector(vector_value(idx));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                let node_id = mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
                if update_ids.len() < WRITES_PER_CYCLE {
                    update_ids.push(node_id);
                }
            }
            mutator
                .create_vector_index(
                    label.clone(),
                    embedding_key.clone(),
                    VectorIndexKind::IvfCosine,
                    DIMENSION as u32,
                )
                .expect("bench vector index build succeeds");
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        Self {
            shared,
            label,
            embedding_key,
            queries: (0..READS_PER_CYCLE).map(vector_value).collect(),
            update_ids,
            update_vectors: (0..WRITES_PER_CYCLE)
                .map(|idx| vector_value(scale + idx + 1))
                .collect(),
        }
    }

    fn run_cycle(self) -> usize {
        let mut read_idx = 0;
        let mut write_idx = 0;
        let mut observed = 0;
        for slot in 0..OPS_PER_CYCLE {
            if slot % 5 < 3 {
                observed += self.read_once(read_idx);
                read_idx += 1;
            } else {
                self.write_once(write_idx);
                write_idx += 1;
            }
        }
        observed
    }

    fn read_once(&self, idx: usize) -> usize {
        let options =
            ApproximateVectorSearchOptions::new(VectorMetric::Cosine, TOP_K, IVF_SEARCH_WIDTH);
        self.shared
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &self.queries[idx],
                options,
                CancellationChecker::disabled(),
            )
            .expect("bench ANN search succeeds")
            .len()
    }

    fn write_once(&self, idx: usize) {
        let mut txn = self.shared.begin_write();
        let mut mutator = txn.mutator();
        let diff = PropertyDiff::new(
            [(
                self.embedding_key.clone(),
                Value::Vector(self.update_vectors[idx].clone()),
            )],
            [],
        )
        .expect("bench property diff is valid");
        mutator
            .update_node(
                self.update_ids[idx],
                LabelDiff::new([], []).expect("bench label diff is valid"),
                diff,
            )
            .expect("bench vector update succeeds");
        txn.commit().expect("bench vector update commit succeeds");
    }
}

fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(|raw| {
            let mut scales: Vec<_> = raw
                .split(',')
                .filter_map(|part| part.trim().parse::<usize>().ok())
                .filter(|scale| *scale > 0)
                .collect();
            scales.sort_unstable();
            scales.dedup();
            (!scales.is_empty()).then_some(scales)
        })
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn vector_value(seed: usize) -> VectorValue {
    VectorValue::new(vector_components(seed)).expect("bench vector is valid")
}

fn vector_components(seed: usize) -> Vec<f32> {
    (0..DIMENSION)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect()
}

criterion_group! {
    name = vector_mixed_workload;
    config = common::criterion_config();
    targets = bench_vector_mixed_workload
}
criterion_main!(vector_mixed_workload);
