#![allow(missing_docs)]
//! Criterion benches for single-snapshot graph read hot paths.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
mod single_graph_ann_recall;
mod single_graph_candidate_set;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, VectorMetric, VectorValue, intern};
use selene_graph::{SeleneGraph, SharedGraph, VectorIndexKind, VectorIndexMemoryUsage};
use selene_testing::BenchProfile;
use single_graph_ann_recall::{ANN_RECALL_PROFILES, AnnRecallFixture};

const ANN_RECALL_K: usize = 10;
const ANN_RECALL_QUERIES: usize = 16;

fn bench_node_fetch(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_node_fetch");
    for fixture in common::fixtures() {
        group.throughput(Throughput::Elements(fixture.scale() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let props = fixture.graph().node_properties(fixture.sample_node_id());
                    std::hint::black_box(props.map(selene_core::PropertyMap::len));
                });
            },
        );
    }
    group.finish();
}

fn bench_label_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_label_index_lookup");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let rows = fixture.graph().nodes_with_label(&fixture.sensor_label());
                    std::hint::black_box(rows.map(roaring::RoaringBitmap::len));
                });
            },
        );
    }
    group.finish();
}

fn bench_typed_index_point(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_typed_index_point");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let rows = fixture.graph().nodes_with_property_eq(
                        &fixture.person_label(),
                        &fixture.age_key(),
                        &Value::Int(fixture.sample_age_value()),
                    );
                    std::hint::black_box(rows.map(|rows| rows.len()));
                });
            },
        );
    }
    group.finish();
}

fn bench_typed_index_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_typed_index_range");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let start = Value::Int(fixture.sample_age_value());
                    let end = Value::Int(fixture.sample_age_value() + 20);
                    let rows = fixture.graph().nodes_with_property_range(
                        &fixture.person_label(),
                        &fixture.age_key(),
                        start..end,
                    );
                    std::hint::black_box(rows.map(|rows| rows.len()));
                });
            },
        );
    }
    group.finish();
}

fn bench_composite_index_proxy(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_composite_index_proxy");
    for fixture in common::fixtures() {
        group.bench_with_input(
            BenchmarkId::from_parameter(fixture.scale()),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let age_rows = fixture
                        .graph()
                        .nodes_with_property_eq(
                            &fixture.person_label(),
                            &fixture.age_key(),
                            &Value::Int(fixture.sample_age_value()),
                        )
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    let mut name_rows = fixture
                        .graph()
                        .nodes_with_property_eq(
                            &fixture.person_label(),
                            &fixture.name_key(),
                            &Value::String(fixture.sample_name_value()),
                        )
                        .map(std::borrow::Cow::into_owned)
                        .unwrap_or_default();
                    name_rows &= age_rows;
                    std::hint::black_box(name_rows.len());
                });
            },
        );
    }
    group.finish();
}

fn bench_exact_vector_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_exact_vector_scan");
    for scale in vector_scan_scales() {
        for &(metric_name, metric) in &[
            ("squared_euclidean", VectorMetric::SquaredEuclidean),
            ("cosine", VectorMetric::Cosine),
        ] {
            for &(index_name, index_kind) in &[
                ("unindexed", None),
                ("flat_index", Some(VectorIndexKind::Flat)),
            ] {
                let fixture = VectorFixture::build(scale, 128, index_kind);
                let memory_suffix = fixture.memory_id_suffix();
                group.throughput(Throughput::Elements(fixture.scale() as u64));
                group.bench_with_input(
                    BenchmarkId::new(
                        format!("{index_name}_{metric_name}_dim128_k10_{memory_suffix}"),
                        fixture.scale(),
                    ),
                    &fixture,
                    |b, fixture| {
                        b.iter(|| {
                            let hits = fixture
                                .graph()
                                .exact_vector_search_nodes(
                                    &fixture.label(),
                                    &fixture.embedding_key(),
                                    fixture.query(),
                                    metric,
                                    10,
                                )
                                .expect("fixture vectors have matching dimensions");
                            std::hint::black_box(hits.len());
                        });
                    },
                );
            }
        }
    }
    group.finish();
}

fn bench_ann_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_ann_recall_validation");
    for scale in vector_scan_scales() {
        for &profile in ANN_RECALL_PROFILES {
            for &variant in profile.variants() {
                let fixture = AnnRecallFixture::build(
                    profile,
                    variant,
                    scale,
                    ANN_RECALL_QUERIES,
                    ANN_RECALL_K,
                );
                let memory_suffix = format_vector_memory_id_suffix(fixture.memory_usage());
                let profile_name = ann_recall_benchmark_name(&fixture);
                group.throughput(Throughput::Elements(
                    (fixture.scale() * fixture.query_count()) as u64,
                ));
                for &ef_search in variant.search_widths() {
                    let recall = fixture.mean_recall(ef_search);
                    let quality = fixture.mean_distance_quality(ef_search);
                    group.bench_with_input(
                        BenchmarkId::new(
                            format!(
                                "{profile_name}_d{}_k{ANN_RECALL_K}_ef{ef_search}_idbp{}_dqbp{}_{}",
                                fixture.dimension(),
                                recall_basis_points(recall),
                                recall_basis_points(quality),
                                memory_suffix
                            ),
                            fixture.scale(),
                        ),
                        &fixture,
                        |b, fixture| {
                            b.iter(|| {
                                let overlap = fixture.total_overlap(ef_search);
                                std::hint::black_box(overlap);
                            });
                        },
                    );
                }
            }
        }
    }
    group.finish();
}

fn ann_recall_benchmark_name(fixture: &AnnRecallFixture) -> String {
    if fixture.variant_name_suffix().is_empty() {
        fixture.profile().name().to_owned()
    } else {
        format!(
            "{}_{}",
            fixture.profile().name(),
            fixture.variant_name_suffix()
        )
    }
}

fn vector_scan_scales() -> Vec<usize> {
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

#[derive(Clone, Debug)]
struct VectorFixture {
    scale: usize,
    graph: SeleneGraph,
    label: IStr,
    embedding_key: IStr,
    query: VectorValue,
}

impl VectorFixture {
    fn build(scale: usize, dimension: usize, index_kind: Option<VectorIndexKind>) -> Self {
        let scale = scale.max(1);
        let label = intern("VectorDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(9_000 + scale as u64));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let vector = Value::Vector(vector_value(idx, dimension));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            if let Some(kind) = index_kind {
                let dimension = u32::try_from(dimension).expect("bench dimension fits u32");
                mutator
                    .create_vector_index(label.clone(), embedding_key.clone(), kind, dimension)
                    .expect("bench vector index build succeeds");
            }
            txn.commit().expect("bench vector fixture commit succeeds");
        }
        Self {
            scale,
            graph: shared.read().as_ref().clone(),
            label,
            embedding_key,
            query: vector_value(0, dimension),
        }
    }

    const fn graph(&self) -> &SeleneGraph {
        &self.graph
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    fn label(&self) -> IStr {
        self.label.clone()
    }

    fn embedding_key(&self) -> IStr {
        self.embedding_key.clone()
    }

    const fn query(&self) -> &VectorValue {
        &self.query
    }

    fn memory_id_suffix(&self) -> String {
        vector_index_memory_id_suffix(&self.graph, &self.label, &self.embedding_key)
    }
}

fn vector_index_memory_id_suffix(graph: &SeleneGraph, label: &IStr, property: &IStr) -> String {
    graph
        .vector_index_for(label, property)
        .map(|index| format_vector_memory_id_suffix(index.memory_usage()))
        .unwrap_or_else(|| "noidx".to_owned())
}

fn format_vector_memory_id_suffix(usage: VectorIndexMemoryUsage) -> String {
    let ann_suffix = if usage.hnsw_entries > 0 {
        format!(
            "he{}l{}d{}g{}z{}u{}p{}x{}a{}",
            compact_usize(usage.hnsw_entries),
            compact_usize(usage.hnsw_live_entries),
            compact_usize(usage.hnsw_deleted_entries),
            compact_usize(usage.hnsw_link_count),
            compact_usize(usage.hnsw_level_zero_link_count),
            compact_usize(usage.hnsw_upper_layer_link_count),
            compact_usize(usage.hnsw_max_layer_count),
            compact_usize(usage.hnsw_max_links_per_layer),
            compact_usize(usage.hnsw_average_links_per_entry_basis_points)
        )
    } else if usage.ivf_entries > 0 {
        format!(
            "ve{}l{}d{}c{}q{}a{}",
            compact_usize(usage.ivf_entries),
            compact_usize(usage.ivf_live_entries),
            compact_usize(usage.ivf_deleted_entries),
            compact_usize(usage.ivf_centroids),
            compact_usize(usage.ivf_list_count),
            compact_usize(usage.ivf_assigned_entries)
        )
    } else {
        "flat".to_owned()
    };
    format!(
        "m{}-{}_n{}_{}",
        usage.estimated_index_bytes / 1024,
        usage.estimated_reachable_bytes / 1024,
        compact_count(usage.indexed_rows),
        ann_suffix,
    )
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

fn recall_basis_points(recall: f64) -> u64 {
    (recall * 10_000.0).round() as u64
}

criterion_group! {
    name = graph_reads;
    config = common::criterion_config();
    targets = bench_node_fetch, bench_label_index, bench_typed_index_point,
        bench_typed_index_range, bench_composite_index_proxy, bench_exact_vector_scan,
        single_graph_candidate_set::bench_vector_candidate_set, bench_ann_recall
}
criterion_main!(graph_reads);
