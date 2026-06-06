#![allow(missing_docs)]
//! Mixed vector read/write workload benchmark.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::{hint::black_box, num::NonZeroUsize};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, IStr, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap,
    Value, VectorMetric, VectorValue, intern,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SharedGraph, VectorIndexKind, VectorIndexMaintenancePolicy,
    VectorIndexRebuildReport,
};
use selene_testing::BenchProfile;

const DIMENSION: usize = 128;
const READS_PER_CYCLE: usize = 60;
const WRITES_PER_CYCLE: usize = 40;
const OPS_PER_CYCLE: usize = READS_PER_CYCLE + WRITES_PER_CYCLE;
const TOP_K: usize = 10;
const IVF_SEARCH_WIDTH: usize = 2;
const MAINTENANCE_INDEXES: usize = 4;
const MAINTENANCE_CYCLES: usize = 10;
const WRITES_PER_INDEX_PER_CYCLE: usize = WRITES_PER_CYCLE / MAINTENANCE_INDEXES;
const UPDATES_PER_MAINTENANCE_INDEX: usize = WRITES_PER_INDEX_PER_CYCLE * MAINTENANCE_CYCLES;
const MAINTENANCE_LABELS: [&str; MAINTENANCE_INDEXES] = [
    "MaintVectorDocA",
    "MaintVectorDocB",
    "MaintVectorDocC",
    "MaintVectorDocD",
];

fn bench_vector_mixed_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_mixed_workload");
    for scale in vector_scales() {
        group.throughput(Throughput::Elements(OPS_PER_CYCLE as u64));
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
        group.bench_with_input(
            BenchmarkId::new("point_read_ivf_update_r60w40_dim128", scale),
            &scale,
            |b, &scale| {
                b.iter_batched(
                    || MixedVectorFixture::build(scale),
                    |fixture| black_box(fixture.run_point_read_update_cycle()),
                    BatchSize::LargeInput,
                );
            },
        );
        group.throughput(Throughput::Elements(
            (OPS_PER_CYCLE * MAINTENANCE_CYCLES) as u64,
        ));
        for mode in [MaintenanceMode::CapOne, MaintenanceMode::Unlimited] {
            group.bench_with_input(
                BenchmarkId::new(mode.benchmark_name(), scale),
                &(scale, mode),
                |b, &(scale, mode)| {
                    b.iter_batched(
                        || MixedMaintenanceFixture::build(scale),
                        |fixture| black_box(fixture.run_cycles_with_maintenance(mode)),
                        BatchSize::LargeInput,
                    );
                },
            );
        }
    }
    group.finish();
}

struct MixedVectorFixture {
    shared: SharedGraph,
    label: IStr,
    embedding_key: IStr,
    queries: Vec<VectorValue>,
    read_ids: Vec<NodeId>,
    update_ids: Vec<NodeId>,
    update_vectors: Vec<VectorValue>,
}

impl MixedVectorFixture {
    fn build(scale: usize) -> Self {
        let scale = scale.max(READS_PER_CYCLE);
        let label = intern("VectorDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(12_000 + scale as u64));
        let mut read_ids = Vec::with_capacity(READS_PER_CYCLE);
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
                if read_ids.len() < READS_PER_CYCLE {
                    read_ids.push(node_id);
                }
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
            read_ids,
            update_ids,
            update_vectors: (0..WRITES_PER_CYCLE)
                .map(|idx| vector_value(scale + idx + 1))
                .collect(),
        }
    }

    fn run_cycle(self) -> usize {
        self.run_cycle_with_reads(VectorReadMode::Ann)
    }

    fn run_point_read_update_cycle(self) -> usize {
        self.run_cycle_with_reads(VectorReadMode::Point)
    }

    fn run_cycle_with_reads(self, mode: VectorReadMode) -> usize {
        let mut read_idx = 0;
        let mut write_idx = 0;
        let mut observed = 0;
        for slot in 0..OPS_PER_CYCLE {
            if slot % 5 < 3 {
                observed += mode.read_once(&self, read_idx);
                read_idx += 1;
            } else {
                self.write_once(write_idx);
                write_idx += 1;
            }
        }
        observed
    }

    fn read_ann_once(&self, idx: usize) -> usize {
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

    fn read_point_once(&self, idx: usize) -> usize {
        self.shared
            .read()
            .node_properties(self.read_ids[idx])
            .is_some() as usize
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

#[derive(Clone, Copy)]
enum VectorReadMode {
    Ann,
    Point,
}

impl VectorReadMode {
    fn read_once(self, fixture: &MixedVectorFixture, idx: usize) -> usize {
        match self {
            Self::Ann => fixture.read_ann_once(idx),
            Self::Point => fixture.read_point_once(idx),
        }
    }
}

struct MixedMaintenanceFixture {
    shared: SharedGraph,
    labels: Vec<IStr>,
    embedding_key: IStr,
    queries: Vec<VectorValue>,
    update_ids: Vec<Vec<NodeId>>,
    update_vectors: Vec<Vec<VectorValue>>,
}

impl MixedMaintenanceFixture {
    fn build(scale: usize) -> Self {
        let scale = scale.max(UPDATES_PER_MAINTENANCE_INDEX);
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(13_000 + scale as u64));
        let labels = MAINTENANCE_LABELS
            .iter()
            .map(|label| intern(label).expect("bench label is valid"))
            .collect::<Vec<_>>();
        let mut update_ids = vec![Vec::with_capacity(UPDATES_PER_MAINTENANCE_INDEX); labels.len()];
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for (label_idx, label) in labels.iter().enumerate() {
                for idx in 0..scale {
                    let vector = Value::Vector(vector_value(label_idx * 100_000 + idx));
                    let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                        .expect("bench vector properties are valid");
                    let node_id = mutator
                        .create_node(LabelSet::single(label.clone()), props)
                        .expect("bench vector node insert succeeds");
                    if update_ids[label_idx].len() < UPDATES_PER_MAINTENANCE_INDEX {
                        update_ids[label_idx].push(node_id);
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
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        let update_vectors = (0..labels.len())
            .map(|label_idx| {
                (0..UPDATES_PER_MAINTENANCE_INDEX)
                    .map(|idx| vector_value(scale + label_idx * 100_000 + idx + 1))
                    .collect()
            })
            .collect();
        Self {
            shared,
            labels,
            embedding_key,
            queries: (0..(READS_PER_CYCLE * MAINTENANCE_CYCLES))
                .map(vector_value)
                .collect(),
            update_ids,
            update_vectors,
        }
    }

    fn run_cycles_with_maintenance(self, mode: MaintenanceMode) -> usize {
        let mut read_idx = 0;
        let mut write_idx = 0;
        let mut observed = 0;
        for _ in 0..MAINTENANCE_CYCLES {
            for slot in 0..OPS_PER_CYCLE {
                if slot % 5 < 3 {
                    observed += self.read_once(read_idx);
                    read_idx += 1;
                } else {
                    self.write_once(write_idx);
                    write_idx += 1;
                }
            }
        }
        let report = mode
            .maintain(&self.shared)
            .expect("bench vector maintenance succeeds");
        mode.validate_report(&report);
        observed + report.indexes_rebuilt
    }

    fn read_once(&self, idx: usize) -> usize {
        let label_idx = idx % self.labels.len();
        let options =
            ApproximateVectorSearchOptions::new(VectorMetric::Cosine, TOP_K, IVF_SEARCH_WIDTH);
        self.shared
            .approximate_vector_search_nodes_checked(
                &self.labels[label_idx],
                &self.embedding_key,
                &self.queries[idx],
                options,
                CancellationChecker::disabled(),
            )
            .expect("bench ANN search succeeds")
            .len()
    }

    fn write_once(&self, idx: usize) {
        let label_idx = idx % self.labels.len();
        let local_idx = idx / self.labels.len();
        let mut txn = self.shared.begin_write();
        let mut mutator = txn.mutator();
        let diff = PropertyDiff::new(
            [(
                self.embedding_key.clone(),
                Value::Vector(self.update_vectors[label_idx][local_idx].clone()),
            )],
            [],
        )
        .expect("bench property diff is valid");
        mutator
            .update_node(
                self.update_ids[label_idx][local_idx],
                LabelDiff::new([], []).expect("bench label diff is valid"),
                diff,
            )
            .expect("bench vector update succeeds");
        txn.commit().expect("bench vector update commit succeeds");
    }
}

#[derive(Clone, Copy)]
enum MaintenanceMode {
    CapOne,
    Unlimited,
}

impl MaintenanceMode {
    fn benchmark_name(self) -> &'static str {
        match self {
            Self::CapOne => "ivf_cos_dim128_k10_r60w40x10_ef2_maint_cap1",
            Self::Unlimited => "ivf_cos_dim128_k10_r60w40x10_ef2_maint_all",
        }
    }

    fn maintain(self, shared: &SharedGraph) -> selene_graph::GraphResult<VectorIndexRebuildReport> {
        match self {
            Self::CapOne => shared.maintain_vector_indexes(
                VectorIndexMaintenancePolicy::recommended()
                    .with_max_indexes_per_run(NonZeroUsize::new(1).expect("one is non-zero")),
            ),
            Self::Unlimited => shared.rebuild_recommended_vector_indexes(),
        }
    }

    fn validate_report(self, report: &VectorIndexRebuildReport) {
        let expected = match self {
            Self::CapOne => 1,
            Self::Unlimited => MAINTENANCE_INDEXES,
        };
        assert_eq!(report.indexes_rebuilt, expected);
        assert_eq!(report.entries.len(), expected);
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.before.ivf_rebuild_recommended())
        );
        assert!(
            report
                .entries
                .iter()
                .all(|entry| !entry.after.ivf_rebuild_recommended())
        );
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
