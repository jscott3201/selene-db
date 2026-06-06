use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput};
use selene_core::{DbString, GraphId, LabelSet, PropertyMap, Value, db_string};
use selene_graph::{SharedGraph, VectorIndexRebuildEntry, VectorIndexRebuildReport};

use super::{
    VECTOR_DIMENSION, VectorRebuildVariant, compact_usize, seed_indexed_nodes,
    vector_rebuild_group_enabled, vector_rebuild_scales, vector_rebuild_variants, vector_value,
};

const COLD_INDEXES: usize = 3;
const COLD_LABELS: [&str; COLD_INDEXES] = [
    "RecommendedColdVectorDocA",
    "RecommendedColdVectorDocB",
    "RecommendedColdVectorDocC",
];

pub(super) fn bench_vector_index_recommended_rebuild(c: &mut Criterion) {
    if !vector_rebuild_group_enabled("recommended_rebuild") {
        return;
    }
    let mut group = c.benchmark_group("graph_vector_index_recommended_rebuild");
    let variants = vector_rebuild_variants()
        .into_iter()
        .filter(|variant| variant.kind.ivf_metric().is_some())
        .collect::<Vec<_>>();
    assert!(
        !variants.is_empty(),
        "recommended rebuild benchmark has no IVF variants"
    );
    for scale in vector_rebuild_scales() {
        for variant in &variants {
            let variant = *variant;
            bench_recommended(&mut group, scale, variant);
            bench_full(&mut group, scale, variant);
        }
    }
    group.finish();
}

fn bench_recommended(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    variant: VectorRebuildVariant,
) {
    let preview = RecommendedRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
    let preview_report = preview
        .shared
        .rebuild_recommended_vector_indexes()
        .expect("bench preview recommended rebuild succeeds");
    preview.validate_recommended_report(&preview_report);
    let id_suffix = preview.format_report_id_suffix(&preview_report);
    group.throughput(Throughput::Elements(report_indexed_rows(&preview_report)));
    group.bench_function(
        BenchmarkId::new(
            format!("{}_recommended", variant.name),
            format!("n{}_{}", compact_usize(scale), id_suffix),
        ),
        |b| {
            b.iter_custom(|iterations| timed_recommended(iterations, scale, variant));
        },
    );
}

fn bench_full(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    scale: usize,
    variant: VectorRebuildVariant,
) {
    let preview = RecommendedRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
    let preview_report = preview
        .shared
        .rebuild_vector_indexes()
        .expect("bench preview full rebuild succeeds");
    preview.validate_full_report(&preview_report);
    let id_suffix = preview.format_report_id_suffix(&preview_report);
    group.throughput(Throughput::Elements(report_indexed_rows(&preview_report)));
    group.bench_function(
        BenchmarkId::new(
            format!("{}_full", variant.name),
            format!("n{}_{}", compact_usize(scale), id_suffix),
        ),
        |b| {
            b.iter_custom(|iterations| timed_full(iterations, scale, variant));
        },
    );
}

fn timed_recommended(iterations: u64, scale: usize, variant: VectorRebuildVariant) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let fixture = RecommendedRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
        let started = Instant::now();
        let report = fixture
            .shared
            .rebuild_recommended_vector_indexes()
            .expect("bench recommended rebuild succeeds");
        elapsed += started.elapsed();
        fixture.validate_recommended_report(&report);
        std::hint::black_box(report.indexes_rebuilt);
    }
    elapsed
}

fn timed_full(iterations: u64, scale: usize, variant: VectorRebuildVariant) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iterations {
        let fixture = RecommendedRebuildFixture::build(scale, VECTOR_DIMENSION, variant);
        let started = Instant::now();
        let report = fixture
            .shared
            .rebuild_vector_indexes()
            .expect("bench full rebuild succeeds");
        elapsed += started.elapsed();
        fixture.validate_full_report(&report);
        std::hint::black_box(report.indexes_rebuilt);
    }
    elapsed
}

fn report_indexed_rows(report: &VectorIndexRebuildReport) -> u64 {
    report
        .entries
        .iter()
        .map(|entry| entry.before.indexed_rows)
        .sum()
}

struct RecommendedRebuildFixture {
    shared: SharedGraph,
    variant: VectorRebuildVariant,
    hot_label: DbString,
    embedding_key: DbString,
    total_indexes: usize,
}

impl RecommendedRebuildFixture {
    fn build(scale: usize, dimension: usize, variant: VectorRebuildVariant) -> Self {
        let scale = scale.max(100);
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(70_000_000 + scale as u64));
        let hot_label = db_string("RecommendedHotVectorDoc").expect("bench label is valid");
        let _hot_ids = seed_indexed_nodes(
            &shared,
            &hot_label,
            &embedding_key,
            scale,
            dimension,
            variant,
        );
        insert_indexed_nodes(
            &shared,
            &hot_label,
            &embedding_key,
            (scale / 10).max(100),
            scale,
            dimension,
        );
        for label in COLD_LABELS {
            let label = db_string(label).expect("bench label is valid");
            let _cold_ids =
                seed_indexed_nodes(&shared, &label, &embedding_key, scale, dimension, variant);
        }
        Self {
            shared,
            variant,
            hot_label,
            embedding_key,
            total_indexes: COLD_INDEXES + 1,
        }
    }

    fn validate_recommended_report(&self, report: &VectorIndexRebuildReport) {
        assert_eq!(report.indexes_rebuilt, 1);
        assert_eq!(report.entries.len(), 1);
        self.validate_hot_entry(&report.entries[0]);
    }

    fn validate_full_report(&self, report: &VectorIndexRebuildReport) {
        assert_eq!(report.indexes_rebuilt, self.total_indexes);
        assert_eq!(report.entries.len(), self.total_indexes);
        let hot = report
            .entries
            .iter()
            .find(|entry| entry.label == self.hot_label)
            .expect("full rebuild report contains hot index");
        self.validate_hot_entry(hot);
        assert!(
            report
                .entries
                .iter()
                .filter(|entry| entry.label != self.hot_label)
                .all(|entry| !entry.before.ivf_rebuild_recommended())
        );
    }

    fn validate_hot_entry(&self, entry: &VectorIndexRebuildEntry) {
        assert_eq!(entry.kind, self.variant.kind);
        assert_eq!(entry.property, self.embedding_key);
        assert_eq!(entry.label, self.hot_label);
        assert!(entry.before.ivf_rebuild_recommended());
        assert!(!entry.after.ivf_rebuild_recommended());
        assert_eq!(entry.after.ivf_pending_retrain_entries, 0);
    }

    fn format_report_id_suffix(&self, report: &VectorIndexRebuildReport) -> String {
        let hot = report
            .entries
            .iter()
            .find(|entry| entry.label == self.hot_label)
            .expect("report contains hot index");
        format!(
            "idx{}_rb{}_pend{}_bp{}",
            self.total_indexes,
            report.indexes_rebuilt,
            compact_usize(hot.before.ivf_pending_retrain_entries),
            hot.before.ivf_pending_retrain_basis_points()
        )
    }
}

fn insert_indexed_nodes(
    shared: &SharedGraph,
    label: &DbString,
    embedding_key: &DbString,
    count: usize,
    seed_offset: usize,
    dimension: usize,
) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for idx in 0..count {
        let props = PropertyMap::from_pairs([(
            embedding_key.clone(),
            Value::Vector(vector_value(seed_offset + idx, dimension)),
        )])
        .expect("bench vector properties are valid");
        mutator
            .create_node(LabelSet::single(label.clone()), props)
            .expect("bench post-index vector node insert succeeds");
    }
    txn.commit().expect("bench post-index insert commits");
}
