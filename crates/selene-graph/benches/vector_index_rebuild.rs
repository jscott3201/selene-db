#![allow(missing_docs)]
//! Criterion benches for vector-index maintenance rebuilds.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, IStr, LabelDiff, LabelSet, PropertyDiff, PropertyMap, Value,
    VectorValue, intern,
};
use selene_graph::{
    ApproximateVectorSearchOptions, HnswIndexConfig, SharedGraph, VectorIndexKind,
    VectorIndexMemoryUsage, VectorIndexRebuildReport,
};
use selene_testing::BenchProfile;

const VECTOR_DIMENSION: usize = 128;
const STALE_QUERY_K: usize = 10;
const STALE_QUERY_EF_SEARCH: usize = 64;
const MEMORY_PROJECTION_K: usize = 10;
const MEMORY_PROJECTION_EF_SEARCH: usize = 64;
const MEMORY_PROJECTION_DIMENSIONS: [usize; 3] = [128, 768, 1536];
const VECTOR_REBUILD_VARIANTS: [VectorRebuildVariant; 6] = [
    VectorRebuildVariant {
        name: "hnsw_l2_dim128_default",
        kind: VectorIndexKind::HnswSquaredEuclidean,
        hnsw_config: None,
    },
    VectorRebuildVariant {
        name: "hnsw_l2_dim128_m24ef64",
        kind: VectorIndexKind::HnswSquaredEuclidean,
        hnsw_config: Some(HnswIndexConfig::new(24, 64)),
    },
    VectorRebuildVariant {
        name: "hnsw_cos_dim128_default",
        kind: VectorIndexKind::HnswCosine,
        hnsw_config: None,
    },
    VectorRebuildVariant {
        name: "hnsw_cos_dim128_m24ef64",
        kind: VectorIndexKind::HnswCosine,
        hnsw_config: Some(HnswIndexConfig::new(24, 64)),
    },
    VectorRebuildVariant {
        name: "ivf_l2_dim128",
        kind: VectorIndexKind::IvfSquaredEuclidean,
        hnsw_config: None,
    },
    VectorRebuildVariant {
        name: "ivf_cos_dim128",
        kind: VectorIndexKind::IvfCosine,
        hnsw_config: None,
    },
];

#[derive(Clone, Copy)]
struct VectorRebuildVariant {
    name: &'static str,
    kind: VectorIndexKind,
    hnsw_config: Option<HnswIndexConfig>,
}

impl VectorRebuildVariant {
    fn metric(self) -> selene_core::VectorMetric {
        self.kind
            .ann_metric()
            .expect("bench variants are ANN indexes")
    }

    fn expected_hnsw_config(self) -> Option<HnswIndexConfig> {
        self.kind
            .hnsw_metric()
            .map(|_| self.hnsw_config.unwrap_or_default())
    }

    fn is_hnsw(self) -> bool {
        self.kind.hnsw_metric().is_some()
    }
}

fn bench_vector_index_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_index_rebuild");
    for scale in vector_rebuild_scales() {
        for variant in VECTOR_REBUILD_VARIANTS {
            let preview = VectorRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
            let preview_report = preview
                .shared
                .rebuild_vector_indexes()
                .expect("bench preview rebuild succeeds");
            preview.validate_report(&preview_report);
            let id_suffix = preview.format_report_id_suffix(&preview_report);
            group.throughput(Throughput::Elements(
                u64::try_from(usage_entries(preview_report.entries[0].before, variant))
                    .unwrap_or(u64::MAX),
            ));
            group.bench_function(
                BenchmarkId::new(
                    variant.name,
                    format!("n{}_{}", compact_usize(scale), id_suffix),
                ),
                |b| {
                    b.iter_custom(|iterations| {
                        let mut elapsed = Duration::ZERO;
                        for _ in 0..iterations {
                            let fixture =
                                VectorRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
                            let started = Instant::now();
                            let report = fixture
                                .shared
                                .rebuild_vector_indexes()
                                .expect("bench vector-index rebuild succeeds");
                            elapsed += started.elapsed();
                            fixture.validate_report(&report);
                            std::hint::black_box(report.reclaimed_reachable_bytes);
                        }
                        elapsed
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_vector_index_stale_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_index_stale_query");
    for scale in vector_rebuild_scales() {
        for variant in VECTOR_REBUILD_VARIANTS {
            let stale = VectorRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
            let stale_suffix = format_usage_id_suffix(stale.memory_usage());
            group.bench_function(
                BenchmarkId::new(
                    variant.name,
                    format!("stale_n{}_{}", compact_usize(scale), stale_suffix),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(stale.approximate_query_hit_count());
                    });
                },
            );

            let rebuilt = VectorRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
            let report = rebuilt
                .shared
                .rebuild_vector_indexes()
                .expect("bench query rebuild succeeds");
            rebuilt.validate_report(&report);
            let rebuilt_suffix = format_usage_id_suffix(rebuilt.memory_usage());
            group.bench_function(
                BenchmarkId::new(
                    variant.name,
                    format!("rebuilt_n{}_{}", compact_usize(scale), rebuilt_suffix),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(rebuilt.approximate_query_hit_count());
                    });
                },
            );
        }
    }
    group.finish();
}

fn bench_vector_index_dimension_projection(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_vector_index_dimension_projection");
    for scale in vector_rebuild_scales() {
        for dimension in MEMORY_PROJECTION_DIMENSIONS {
            let fixture = VectorMemoryProjectionFixture::build(scale, dimension);
            let memory_suffix = format_usage_id_suffix(fixture.memory_usage());
            group.bench_function(
                BenchmarkId::new(
                    format!("hnsw_l2_default_dim{dimension}"),
                    format!("n{}_{}", compact_usize(scale), memory_suffix),
                ),
                |b| {
                    b.iter(|| {
                        std::hint::black_box(fixture.approximate_query_hit_count());
                    });
                },
            );
        }
    }
    group.finish();
}

fn vector_rebuild_scales() -> Vec<usize> {
    std::env::var("SELENE_VECTOR_REBUILD_BENCH_SCALES")
        .ok()
        .and_then(parse_scales)
        .unwrap_or_else(|| BenchProfile::from_env().scales().to_vec())
}

fn parse_scales(raw: String) -> Option<Vec<usize>> {
    let mut scales: Vec<_> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<usize>().ok())
        .filter(|scale| *scale > 0)
        .collect();
    scales.sort_unstable();
    scales.dedup();
    (!scales.is_empty()).then_some(scales)
}

struct VectorRebuildFixture {
    shared: SharedGraph,
    variant: VectorRebuildVariant,
    label: IStr,
    embedding_key: IStr,
    query: VectorValue,
    scale: usize,
    update_count: usize,
    delete_count: usize,
}

impl VectorRebuildFixture {
    fn build(scale: usize, dimension: usize, variant: VectorRebuildVariant) -> Self {
        let scale = scale.max(2);
        let label = intern("VectorIndexRebuildDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(50_000_000 + scale as u64));
        let ids = seed_indexed_nodes(&shared, &label, &embedding_key, scale, dimension, variant);
        let (update_count, delete_count) = churn_counts(scale);
        churn_indexed_nodes(
            &shared,
            &embedding_key,
            &ids,
            update_count,
            delete_count,
            dimension,
        );
        Self {
            shared,
            variant,
            label,
            embedding_key,
            query: vector_value(scale - 1, dimension),
            scale,
            update_count,
            delete_count,
        }
    }

    fn validate_report(&self, report: &VectorIndexRebuildReport) {
        assert_eq!(report.indexes_rebuilt, 1);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(
            reclaimed_deleted_entries(report, self.variant),
            self.update_count + self.delete_count
        );
        assert!(report.reclaimed_reachable_bytes > 0);

        let live_rows = self.scale - self.delete_count;
        let live_rows_u64 = u64::try_from(live_rows).expect("bench scale fits u64");
        let entry = &report.entries[0];
        assert_eq!(entry.kind, self.variant.kind);
        assert_eq!(entry.hnsw_config, self.variant.expected_hnsw_config());
        assert_eq!(
            usize::try_from(entry.dimension).expect("dimension fits usize"),
            VECTOR_DIMENSION
        );
        assert_eq!(entry.before.indexed_rows, live_rows_u64);
        assert_eq!(usage_live_entries(entry.before, self.variant), live_rows);
        assert_eq!(
            usage_deleted_entries(entry.before, self.variant),
            self.update_count + self.delete_count
        );
        assert_eq!(entry.after.indexed_rows, live_rows_u64);
        assert_eq!(usage_entries(entry.after, self.variant), live_rows);
        assert_eq!(usage_live_entries(entry.after, self.variant), live_rows);
        assert_eq!(usage_deleted_entries(entry.after, self.variant), 0);
        if self.variant.is_hnsw() {
            assert_eq!(
                entry
                    .before
                    .hnsw_level_zero_link_count
                    .saturating_add(entry.before.hnsw_upper_layer_link_count),
                entry.before.hnsw_link_count
            );
            assert_eq!(
                entry
                    .after
                    .hnsw_level_zero_link_count
                    .saturating_add(entry.after.hnsw_upper_layer_link_count),
                entry.after.hnsw_link_count
            );
        }
    }

    fn format_report_id_suffix(&self, report: &VectorIndexRebuildReport) -> String {
        let entry = &report.entries[0];
        format!(
            "upd{}_del{}_b{}-{}-{}_a{}-{}-{}_rk{}",
            compact_usize(self.update_count),
            compact_usize(self.delete_count),
            compact_usize(usage_entries(entry.before, self.variant)),
            compact_usize(usage_live_entries(entry.before, self.variant)),
            compact_usize(usage_deleted_entries(entry.before, self.variant)),
            compact_usize(usage_entries(entry.after, self.variant)),
            compact_usize(usage_live_entries(entry.after, self.variant)),
            compact_usize(usage_deleted_entries(entry.after, self.variant)),
            report.reclaimed_reachable_bytes / 1024
        )
    }

    fn memory_usage(&self) -> VectorIndexMemoryUsage {
        self.shared
            .read()
            .vector_index_for(&self.label, &self.embedding_key)
            .expect("bench fixture has vector index")
            .memory_usage()
    }

    fn approximate_query_hit_count(&self) -> usize {
        self.shared
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &self.query,
                ApproximateVectorSearchOptions::new(
                    self.variant.metric(),
                    STALE_QUERY_K,
                    STALE_QUERY_EF_SEARCH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("bench ANN query succeeds")
            .len()
    }
}

struct VectorMemoryProjectionFixture {
    shared: SharedGraph,
    label: IStr,
    embedding_key: IStr,
    query: VectorValue,
}

impl VectorMemoryProjectionFixture {
    fn build(scale: usize, dimension: usize) -> Self {
        let scale = scale.max(1);
        let label = intern("VectorMemoryProjectionDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(60_000_000 + scale as u64 + dimension as u64));
        let variant = VectorRebuildVariant {
            name: "hnsw_l2_default",
            kind: VectorIndexKind::HnswSquaredEuclidean,
            hnsw_config: None,
        };
        let _ids = seed_indexed_nodes(&shared, &label, &embedding_key, scale, dimension, variant);
        Self {
            shared,
            label,
            embedding_key,
            query: vector_value(scale - 1, dimension),
        }
    }

    fn memory_usage(&self) -> VectorIndexMemoryUsage {
        self.shared
            .read()
            .vector_index_for(&self.label, &self.embedding_key)
            .expect("bench fixture has vector index")
            .memory_usage()
    }

    fn approximate_query_hit_count(&self) -> usize {
        self.shared
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &self.query,
                ApproximateVectorSearchOptions::new(
                    VectorIndexKind::HnswSquaredEuclidean
                        .hnsw_metric()
                        .expect("HNSW L2 has metric"),
                    MEMORY_PROJECTION_K,
                    MEMORY_PROJECTION_EF_SEARCH,
                ),
                CancellationChecker::disabled(),
            )
            .expect("bench HNSW query succeeds")
            .len()
    }
}

fn seed_indexed_nodes(
    shared: &SharedGraph,
    label: &IStr,
    embedding_key: &IStr,
    scale: usize,
    dimension: usize,
    variant: VectorRebuildVariant,
) -> Vec<selene_core::NodeId> {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let mut ids = Vec::with_capacity(scale);
    for idx in 0..scale {
        let props = PropertyMap::from_pairs([(
            embedding_key.clone(),
            Value::Vector(vector_value(idx, dimension)),
        )])
        .expect("bench vector properties are valid");
        ids.push(
            mutator
                .create_node(LabelSet::single(label.clone()), props)
                .expect("bench vector node insert succeeds"),
        );
    }
    mutator
        .create_vector_index_named_with_config(
            label.clone(),
            embedding_key.clone(),
            variant.kind,
            u32::try_from(dimension).expect("bench dimension fits u32"),
            None,
            variant.hnsw_config,
        )
        .expect("bench vector index build succeeds");
    txn.commit().expect("bench seed commit succeeds");
    ids
}

fn churn_indexed_nodes(
    shared: &SharedGraph,
    embedding_key: &IStr,
    ids: &[selene_core::NodeId],
    update_count: usize,
    delete_count: usize,
    dimension: usize,
) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for (offset, id) in ids.iter().copied().take(update_count).enumerate() {
        mutator
            .update_node(
                id,
                LabelDiff::new([], []).expect("empty label diff is valid"),
                PropertyDiff::new(
                    [(
                        embedding_key.clone(),
                        Value::Vector(vector_value(ids.len() + offset, dimension)),
                    )],
                    [],
                )
                .expect("bench property diff is valid"),
            )
            .expect("bench vector update succeeds");
    }
    for id in ids.iter().copied().skip(update_count).take(delete_count) {
        mutator
            .delete_node(id)
            .expect("bench vector delete succeeds");
    }
    txn.commit().expect("bench churn commit succeeds");
}

fn churn_counts(scale: usize) -> (usize, usize) {
    let update_count = (scale / 10).max(1).min(scale / 2);
    let delete_count = (scale / 20).max(1).min(scale - update_count);
    (update_count, delete_count)
}

fn compact_usize(value: usize) -> String {
    compact_count(u64::try_from(value).unwrap_or(u64::MAX))
}

fn format_usage_id_suffix(usage: VectorIndexMemoryUsage) -> String {
    let prefix = if usage.hnsw_entries > 0 { "h" } else { "v" };
    format!(
        "{prefix}e{}l{}d{}_m{}-{}",
        compact_usize(if usage.hnsw_entries > 0 {
            usage.hnsw_entries
        } else {
            usage.ivf_entries
        }),
        compact_usize(if usage.hnsw_entries > 0 {
            usage.hnsw_live_entries
        } else {
            usage.ivf_live_entries
        }),
        compact_usize(if usage.hnsw_entries > 0 {
            usage.hnsw_deleted_entries
        } else {
            usage.ivf_deleted_entries
        }),
        usage.estimated_index_bytes / 1024,
        usage.estimated_reachable_bytes / 1024,
    )
}

fn usage_entries(usage: VectorIndexMemoryUsage, variant: VectorRebuildVariant) -> usize {
    if variant.is_hnsw() {
        usage.hnsw_entries
    } else {
        usage.ivf_entries
    }
}

fn usage_live_entries(usage: VectorIndexMemoryUsage, variant: VectorRebuildVariant) -> usize {
    if variant.is_hnsw() {
        usage.hnsw_live_entries
    } else {
        usage.ivf_live_entries
    }
}

fn usage_deleted_entries(usage: VectorIndexMemoryUsage, variant: VectorRebuildVariant) -> usize {
    if variant.is_hnsw() {
        usage.hnsw_deleted_entries
    } else {
        usage.ivf_deleted_entries
    }
}

fn reclaimed_deleted_entries(
    report: &VectorIndexRebuildReport,
    variant: VectorRebuildVariant,
) -> usize {
    if variant.is_hnsw() {
        report.reclaimed_hnsw_deleted_entries
    } else {
        report.reclaimed_ivf_deleted_entries
    }
}

fn compact_count(value: u64) -> String {
    if value >= 1_000 && value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    VectorValue::new(vector_components(seed, dimension)).expect("bench vector is valid")
}

fn vector_components(seed: usize, dimension: usize) -> Vec<f32> {
    (0..dimension)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect()
}

criterion_group! {
    name = vector_index_maintenance;
    config = common::criterion_config();
    targets = bench_vector_index_rebuild, bench_vector_index_stale_query,
        bench_vector_index_dimension_projection
}
criterion_main!(vector_index_maintenance);
