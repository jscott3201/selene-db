#![allow(missing_docs)]
//! Criterion benches for IVF incremental-insert drift diagnostics.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, DbString, GraphId, LabelSet, PropertyMap, Value, VectorMetric,
    VectorValue, db_string,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexKind,
    VectorIndexMemoryUsage, VectorNodeSearchHit,
};
use selene_testing::BenchProfile;

const DIMENSION: usize = 128;
const K: usize = 10;
const QUERY_COUNT: usize = 16;
const BASIS_POINTS_DENOMINATOR: usize = 10_000;
const DEFAULT_DRIFT_BASIS_POINTS: &[usize] = &[1_000];
const SEARCH_WIDTHS: &[usize] = &[2, 64];
const DISTANCE_TIE_EPSILON: f64 = 1e-9;

fn bench_ivf_insert_drift(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ivf_insert_drift");
    for scale in vector_scales() {
        for drift_basis_points in drift_basis_points() {
            for &width in SEARCH_WIDTHS {
                for mode in [DriftMode::Incremental, DriftMode::Rebuilt] {
                    let fixture = IvfInsertDriftFixture::build(scale, mode, drift_basis_points);
                    let usage = fixture.memory_usage();
                    let recall = recall_basis_points(fixture.mean_recall(width));
                    let quality = recall_basis_points(fixture.mean_distance_quality(width));
                    group.throughput(Throughput::Elements(estimated_candidates_per_batch(
                        usage,
                        width,
                        fixture.query_count(),
                    ) as u64));
                    group.bench_function(
                        BenchmarkId::new(
                            format!("ivf_cos_dim{DIMENSION}_w{width}_d{drift_basis_points}bp"),
                            format!(
                                "{}_n{}_i{}_idbp{}_dqbp{}_{}",
                                mode.name(),
                                compact_usize(fixture.scale()),
                                compact_usize(fixture.insert_count()),
                                recall,
                                quality,
                                pressure_suffix(usage, width)
                            ),
                        ),
                        |b| {
                            b.iter(|| {
                                std::hint::black_box(fixture.total_overlap(width));
                            });
                        },
                    );
                }
            }
        }
    }
    group.finish();
}

#[derive(Clone, Copy, Debug)]
enum DriftMode {
    Incremental,
    Rebuilt,
}

impl DriftMode {
    const fn name(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Rebuilt => "rebuilt",
        }
    }
}

struct IvfInsertDriftFixture {
    graph: SeleneGraph,
    label: DbString,
    embedding_key: DbString,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<VectorNodeSearchHit>>,
    scale: usize,
    insert_count: usize,
}

impl IvfInsertDriftFixture {
    fn build(scale: usize, mode: DriftMode, drift_basis_points: usize) -> Self {
        let scale = scale.max(1);
        let insert_count = drift_insert_count(scale, drift_basis_points);
        let label = db_string("VectorIvfInsertDriftDoc").expect("bench label is valid");
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let graph_id = 80_000_000u64
            .saturating_add(scale as u64)
            .saturating_mul(BASIS_POINTS_DENOMINATOR as u64)
            .saturating_add(drift_basis_points as u64);
        let shared = SharedGraph::new(GraphId::new(graph_id));
        seed_base_index(&shared, &label, &embedding_key, scale);
        insert_novel_cluster(&shared, &label, &embedding_key, scale, insert_count);
        if matches!(mode, DriftMode::Rebuilt) {
            shared
                .rebuild_vector_indexes()
                .expect("bench vector-index rebuild succeeds");
        }
        let graph = shared.read().as_ref().clone();
        let queries = (0..QUERY_COUNT)
            .map(|idx| drift_vector_value(scale + idx, 0.0003))
            .collect::<Vec<_>>();
        let exact = queries
            .iter()
            .map(|query| {
                graph
                    .exact_vector_search_nodes(
                        &label,
                        &embedding_key,
                        query,
                        VectorMetric::Cosine,
                        K,
                    )
                    .expect("bench exact vector search succeeds")
            })
            .collect::<Vec<_>>();
        Self {
            graph,
            label,
            embedding_key,
            queries,
            exact,
            scale,
            insert_count,
        }
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    const fn insert_count(&self) -> usize {
        self.insert_count
    }

    fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn memory_usage(&self) -> VectorIndexMemoryUsage {
        self.graph
            .vector_index_for(&self.label, &self.embedding_key)
            .expect("bench fixture has IVF index")
            .memory_usage()
    }

    fn mean_recall(&self, width: usize) -> f64 {
        let expected = self.exact.iter().map(Vec::len).sum::<usize>();
        if expected == 0 {
            return 1.0;
        }
        self.total_overlap(width) as f64 / expected as f64
    }

    fn mean_distance_quality(&self, width: usize) -> f64 {
        let expected = self.exact.iter().map(Vec::len).sum::<usize>();
        if expected == 0 {
            return 1.0;
        }
        self.total_distance_quality(width) as f64 / expected as f64
    }

    fn total_overlap(&self, width: usize) -> usize {
        self.exact
            .iter()
            .zip(self.approximate_batch(width))
            .map(|(exact, approximate)| overlap_count(exact, &approximate))
            .sum()
    }

    fn total_distance_quality(&self, width: usize) -> usize {
        self.exact
            .iter()
            .zip(self.approximate_batch(width))
            .map(|(exact, approximate)| distance_quality_count(exact, &approximate))
            .sum()
    }

    fn approximate_batch(&self, width: usize) -> Vec<Vec<VectorNodeSearchHit>> {
        self.graph
            .approximate_vector_search_nodes_batch_checked(
                &self.label,
                &self.embedding_key,
                &self.queries,
                ApproximateVectorSearchOptions::new(VectorMetric::Cosine, K, width),
                CancellationChecker::disabled(),
            )
            .expect("bench approximate vector batch search succeeds")
    }
}

fn seed_base_index(shared: &SharedGraph, label: &DbString, embedding_key: &DbString, scale: usize) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for idx in 0..scale {
        let props = PropertyMap::from_pairs([(
            embedding_key.clone(),
            Value::Vector(base_vector_value(idx)),
        )])
        .expect("bench vector properties are valid");
        mutator
            .create_node(LabelSet::single(label.clone()), props)
            .expect("bench vector node insert succeeds");
    }
    mutator
        .create_vector_index(
            label.clone(),
            embedding_key.clone(),
            VectorIndexKind::IvfCosine,
            u32::try_from(DIMENSION).expect("bench dimension fits u32"),
        )
        .expect("bench IVF index build succeeds");
    txn.commit().expect("bench seed commit succeeds");
}

fn insert_novel_cluster(
    shared: &SharedGraph,
    label: &DbString,
    embedding_key: &DbString,
    scale: usize,
    insert_count: usize,
) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for offset in 0..insert_count {
        let props = PropertyMap::from_pairs([(
            embedding_key.clone(),
            Value::Vector(drift_vector_value(scale + offset, 0.0)),
        )])
        .expect("bench vector properties are valid");
        mutator
            .create_node(LabelSet::single(label.clone()), props)
            .expect("bench drift node insert succeeds");
    }
    txn.commit().expect("bench drift insert commit succeeds");
}

fn base_vector_value(seed: usize) -> VectorValue {
    let components = (0..DIMENSION)
        .map(|dim| {
            let cluster = seed % 16;
            let primary = if dim == cluster { 1.0 } else { 0.0 };
            let secondary = if dim == (cluster * 7 + 11) % (DIMENSION / 2) {
                0.25
            } else {
                0.0
            };
            let noise = ((seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 997) as f32 / 10_000.0;
            primary + secondary + noise
        })
        .collect::<Vec<_>>();
    VectorValue::new(components).expect("bench base vector is valid")
}

fn drift_vector_value(seed: usize, query_shift: f32) -> VectorValue {
    let components = (0..DIMENSION)
        .map(|dim| {
            let cluster = seed % 8;
            let primary_dim = DIMENSION - 1 - cluster;
            let secondary_dim = DIMENSION / 2 + (cluster * 5 + 3) % (DIMENSION / 2);
            let primary = if dim == primary_dim {
                1.0 + query_shift
            } else {
                0.0
            };
            let secondary = if dim == secondary_dim { 0.4 } else { 0.0 };
            let noise = ((seed.wrapping_mul(43) + dim.wrapping_mul(19)) % 997) as f32 / 20_000.0;
            primary + secondary + noise
        })
        .collect::<Vec<_>>();
    VectorValue::new(components).expect("bench drift vector is valid")
}

fn overlap_count(exact: &[VectorNodeSearchHit], approximate: &[VectorNodeSearchHit]) -> usize {
    exact
        .iter()
        .filter(|expected| {
            approximate
                .iter()
                .any(|hit| hit.node_id == expected.node_id)
        })
        .count()
}

fn distance_quality_count(
    exact: &[VectorNodeSearchHit],
    approximate: &[VectorNodeSearchHit],
) -> usize {
    let Some(threshold) = exact.last().map(|hit| hit.distance + DISTANCE_TIE_EPSILON) else {
        return 0;
    };
    approximate
        .iter()
        .take(exact.len())
        .filter(|hit| hit.distance <= threshold)
        .count()
}

fn pressure_suffix(usage: VectorIndexMemoryUsage, width: usize) -> String {
    format!(
        "lists{}ne{}max{}avg{}avgq{}maxq{}assn{}pend{}pdbp{}m{}-{}",
        compact_usize(usage.ivf_list_count),
        compact_usize(usage.ivf_non_empty_list_count),
        compact_usize(usage.ivf_max_list_len),
        compact_usize(average_list_len(usage)),
        compact_usize(estimated_candidates_per_query(usage, width)),
        compact_usize(worst_case_candidates_per_query(usage, width)),
        compact_usize(usage.ivf_assigned_entries),
        compact_usize(usage.ivf_pending_retrain_entries),
        compact_usize(pending_retrain_basis_points(usage)),
        usage.estimated_index_bytes / 1024,
        usage.estimated_reachable_bytes / 1024,
    )
}

fn estimated_candidates_per_batch(
    usage: VectorIndexMemoryUsage,
    width: usize,
    query_count: usize,
) -> usize {
    estimated_candidates_per_query(usage, width)
        .saturating_mul(query_count)
        .max(1)
}

fn estimated_candidates_per_query(usage: VectorIndexMemoryUsage, width: usize) -> usize {
    usage
        .ivf_average_list_len_basis_points
        .saturating_mul(width.max(1).min(usage.ivf_list_count))
        .saturating_add(9_999)
        / 10_000
}

fn worst_case_candidates_per_query(usage: VectorIndexMemoryUsage, width: usize) -> usize {
    usage
        .ivf_max_list_len
        .saturating_mul(width.max(1).min(usage.ivf_list_count))
}

fn average_list_len(usage: VectorIndexMemoryUsage) -> usize {
    usage
        .ivf_average_list_len_basis_points
        .saturating_add(9_999)
        / 10_000
}

fn pending_retrain_basis_points(usage: VectorIndexMemoryUsage) -> usize {
    usage
        .ivf_pending_retrain_entries
        .saturating_mul(BASIS_POINTS_DENOMINATOR)
        .checked_div(usage.ivf_live_entries)
        .unwrap_or_default()
}

fn drift_insert_count(scale: usize, drift_basis_points: usize) -> usize {
    scale
        .saturating_mul(drift_basis_points)
        .saturating_add(BASIS_POINTS_DENOMINATOR - 1)
        / BASIS_POINTS_DENOMINATOR
}

fn vector_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_BENCH_SCALES")
        .ok()
        .and_then(|raw| parse_positive_usize_list(raw, usize::MAX))
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn drift_basis_points() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_IVF_INSERT_DRIFT_BPS")
        .ok()
        .and_then(|raw| parse_positive_usize_list(raw, BASIS_POINTS_DENOMINATOR))
        .unwrap_or_else(|| DEFAULT_DRIFT_BASIS_POINTS.to_vec())
}

fn parse_positive_usize_list(raw: String, max_value: usize) -> Option<Vec<usize>> {
    let mut values: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|value| (1..=max_value).contains(value))
        .collect();
    values.sort_unstable();
    values.dedup();
    (!values.is_empty()).then_some(values)
}

fn recall_basis_points(recall: f64) -> u64 {
    (recall * 10_000.0).round() as u64
}

fn compact_usize(value: usize) -> String {
    compact_count(u64::try_from(value).unwrap_or(u64::MAX))
}

fn compact_count(value: u64) -> String {
    if value >= 1_000 && value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

criterion_group! {
    name = vector_ivf_insert_drift;
    config = common::criterion_config();
    targets = bench_ivf_insert_drift
}
criterion_main!(vector_ivf_insert_drift);
