#![allow(missing_docs)]
//! Criterion benches for single-snapshot graph read hot paths.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, VectorMetric,
    VectorValue, intern,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SeleneGraph, SharedGraph, VectorIndexKind, VectorNodeSearchHit,
};
use selene_testing::BenchProfile;

const HNSW_RECALL_K: usize = 10;
const HNSW_RECALL_QUERIES: usize = 16;
const HNSW_RECALL_EF_SEARCH: &[usize] = &[10, 32, 64];
const HNSW_RECALL_PROFILES: &[HnswRecallProfile] = &[
    HnswRecallProfile::LineSquaredEuclidean,
    HnswRecallProfile::ClusteredCosine,
    HnswRecallProfile::NegativeInnerProduct,
];

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
                group.throughput(Throughput::Elements(fixture.scale() as u64));
                group.bench_with_input(
                    BenchmarkId::new(
                        format!("{index_name}_{metric_name}_dim128_k10"),
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

fn bench_hnsw_recall(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_hnsw_recall_validation");
    for scale in vector_scan_scales() {
        for &profile in HNSW_RECALL_PROFILES {
            let fixture =
                HnswRecallFixture::build(profile, scale, HNSW_RECALL_QUERIES, HNSW_RECALL_K);
            group.throughput(Throughput::Elements(
                (fixture.scale() * fixture.query_count()) as u64,
            ));
            for &ef_search in HNSW_RECALL_EF_SEARCH {
                let recall = fixture.mean_recall(ef_search);
                let (index_kib, reachable_kib) = fixture.estimated_memory_kib();
                group.bench_with_input(
                    BenchmarkId::new(
                        format!(
                            "{}_d{}_k{HNSW_RECALL_K}_ef{ef_search}_bp{}_idx{}_reach{}",
                            fixture.profile().name(),
                            fixture.dimension(),
                            recall_basis_points(recall),
                            index_kib,
                            reachable_kib
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
    group.finish();
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

#[derive(Clone, Copy, Debug)]
enum HnswRecallProfile {
    LineSquaredEuclidean,
    ClusteredCosine,
    NegativeInnerProduct,
}

impl HnswRecallProfile {
    const fn name(self) -> &'static str {
        match self {
            Self::LineSquaredEuclidean => "line_l2",
            Self::ClusteredCosine => "cluster_cos",
            Self::NegativeInnerProduct => "mips",
        }
    }

    const fn dimension(self) -> usize {
        match self {
            Self::LineSquaredEuclidean | Self::ClusteredCosine => 128,
            Self::NegativeInnerProduct => 64,
        }
    }

    const fn metric(self) -> VectorMetric {
        match self {
            Self::LineSquaredEuclidean => VectorMetric::SquaredEuclidean,
            Self::ClusteredCosine => VectorMetric::Cosine,
            Self::NegativeInnerProduct => VectorMetric::NegativeInnerProduct,
        }
    }

    const fn index_kind(self) -> VectorIndexKind {
        match self {
            Self::LineSquaredEuclidean => VectorIndexKind::HnswSquaredEuclidean,
            Self::ClusteredCosine => VectorIndexKind::HnswCosine,
            Self::NegativeInnerProduct => VectorIndexKind::HnswNegativeInnerProduct,
        }
    }

    const fn graph_id_offset(self) -> u64 {
        match self {
            Self::LineSquaredEuclidean => 0,
            Self::ClusteredCosine => 1_000_000,
            Self::NegativeInnerProduct => 2_000_000,
        }
    }

    fn corpus_value(self, seed: usize, scale: usize) -> VectorValue {
        match self {
            Self::LineSquaredEuclidean => recall_corpus_value(seed, self.dimension()),
            Self::ClusteredCosine => clustered_cosine_value(seed, scale, self.dimension(), 0.0),
            Self::NegativeInnerProduct => mips_corpus_value(seed, scale, self.dimension()),
        }
    }

    fn query_value(self, query_idx: usize, scale: usize, query_count: usize) -> VectorValue {
        match self {
            Self::LineSquaredEuclidean => {
                recall_query_value(query_idx, scale, query_count, self.dimension())
            }
            Self::ClusteredCosine => {
                let cluster_count = recall_cluster_count(scale);
                let cluster = query_idx % cluster_count;
                let seed = cluster + (scale / cluster_count / 2) * cluster_count;
                clustered_cosine_value(
                    seed.min(scale.saturating_sub(1)),
                    scale,
                    self.dimension(),
                    0.0003,
                )
            }
            Self::NegativeInnerProduct => mips_query_value(query_idx, self.dimension()),
        }
    }
}

#[derive(Clone, Debug)]
struct HnswRecallFixture {
    profile: HnswRecallProfile,
    dimension: usize,
    scale: usize,
    graph: SeleneGraph,
    label: IStr,
    embedding_key: IStr,
    queries: Vec<VectorValue>,
    exact: Vec<Vec<NodeId>>,
    k: usize,
}

impl HnswRecallFixture {
    fn build(profile: HnswRecallProfile, scale: usize, query_count: usize, k: usize) -> Self {
        let scale = scale.max(1);
        let dimension = profile.dimension();
        let label = intern("HnswRecallDoc").expect("bench label is valid");
        let embedding_key = intern("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(
            10_000 + scale as u64 + profile.graph_id_offset(),
        ));
        {
            let mut txn = shared.begin_write();
            let mut mutator = txn.mutator();
            for idx in 0..scale {
                let vector = Value::Vector(profile.corpus_value(idx, scale));
                let props = PropertyMap::from_pairs([(embedding_key.clone(), vector)])
                    .expect("bench vector properties are valid");
                mutator
                    .create_node(LabelSet::single(label.clone()), props)
                    .expect("bench vector node insert succeeds");
            }
            let dimension = u32::try_from(dimension).expect("bench dimension fits u32");
            mutator
                .create_vector_index(
                    label.clone(),
                    embedding_key.clone(),
                    profile.index_kind(),
                    dimension,
                )
                .expect("bench HNSW vector index build succeeds");
            txn.commit()
                .expect("bench HNSW recall fixture commit succeeds");
        }
        let graph = shared.read().as_ref().clone();
        let queries: Vec<_> = (0..query_count)
            .map(|idx| profile.query_value(idx, scale, query_count))
            .collect();
        let exact = queries
            .iter()
            .map(|query| {
                graph
                    .exact_vector_search_nodes(&label, &embedding_key, query, profile.metric(), k)
                    .expect("bench exact vector search succeeds")
                    .into_iter()
                    .map(|hit| hit.node_id)
                    .collect()
            })
            .collect();
        Self {
            profile,
            dimension,
            scale,
            graph,
            label,
            embedding_key,
            queries,
            exact,
            k,
        }
    }

    const fn profile(&self) -> HnswRecallProfile {
        self.profile
    }

    const fn dimension(&self) -> usize {
        self.dimension
    }

    const fn scale(&self) -> usize {
        self.scale
    }

    const fn query_count(&self) -> usize {
        self.queries.len()
    }

    fn mean_recall(&self, ef_search: usize) -> f64 {
        let expected = self.exact.iter().map(Vec::len).sum::<usize>();
        if expected == 0 {
            return 1.0;
        }
        self.total_overlap(ef_search) as f64 / expected as f64
    }

    fn total_overlap(&self, ef_search: usize) -> usize {
        self.queries
            .iter()
            .zip(&self.exact)
            .map(|(query, exact)| {
                let approximate = self
                    .graph
                    .approximate_vector_search_nodes_checked(
                        &self.label,
                        &self.embedding_key,
                        query,
                        ApproximateVectorSearchOptions::new(
                            self.profile.metric(),
                            self.k,
                            ef_search,
                        ),
                        CancellationChecker::disabled(),
                    )
                    .expect("bench approximate vector search succeeds");
                overlap_count(exact, &approximate)
            })
            .sum()
    }

    fn estimated_memory_kib(&self) -> (usize, usize) {
        let Some(usage) = self
            .graph
            .vector_index_for(&self.label, &self.embedding_key)
            .map(|index| index.memory_usage())
        else {
            return (0, 0);
        };
        (
            usage.estimated_index_bytes / 1024,
            usage.estimated_reachable_bytes / 1024,
        )
    }
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
}

fn vector_value(seed: usize, dimension: usize) -> VectorValue {
    VectorValue::new(vector_components(seed, dimension)).expect("bench vector is valid")
}

fn recall_query_value(
    query_idx: usize,
    scale: usize,
    query_count: usize,
    dimension: usize,
) -> VectorValue {
    let seed = query_idx
        .saturating_mul(scale.max(1))
        .checked_div(query_count.max(1))
        .unwrap_or(0)
        .min(scale.saturating_sub(1));
    let mut components = recall_vector_components(seed, dimension);
    if let Some(first) = components.first_mut() {
        *first += 0.37;
    }
    VectorValue::new(components).expect("bench recall query is valid")
}

fn recall_corpus_value(seed: usize, dimension: usize) -> VectorValue {
    VectorValue::new(recall_vector_components(seed, dimension))
        .expect("bench recall corpus vector is valid")
}

fn clustered_cosine_value(
    seed: usize,
    scale: usize,
    dimension: usize,
    query_shift: f32,
) -> VectorValue {
    let cluster_count = recall_cluster_count(scale);
    let cluster = seed % cluster_count;
    let ordinal = seed / cluster_count;
    let center = cluster % dimension;
    let second = cluster.wrapping_mul(5).wrapping_add(3) % dimension;
    let spread = ordinal as f32 - (scale / cluster_count / 2) as f32;
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let base = (((cluster + 3) * (dim + 11)) % 17) as f32 / 200.0;
            let primary = if dim == center { 1.0 } else { 0.0 };
            let secondary = if dim == second { 0.25 } else { 0.0 };
            base + primary + secondary + spread * 0.0002 + query_shift
        })
        .collect();
    VectorValue::new(components).expect("bench clustered cosine vector is valid")
}

fn recall_cluster_count(scale: usize) -> usize {
    scale.clamp(1, 16)
}

fn mips_corpus_value(seed: usize, scale: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let trend = seed as f32 / scale.max(1) as f32;
            let local = ((seed * (dim + 13) + dim * 29) % 101) as f32 / 5_000.0;
            trend * (1.0 + dim as f32 / dimension as f32) + local + 0.01
        })
        .collect();
    VectorValue::new(components).expect("bench MIPS corpus vector is valid")
}

fn mips_query_value(query_idx: usize, dimension: usize) -> VectorValue {
    let components: Vec<f32> = (0..dimension)
        .map(|dim| {
            let weight = 1.0 + dim as f32 / dimension as f32;
            let tilt = ((query_idx + dim * 7) % 23) as f32 / 1_000.0;
            weight + tilt
        })
        .collect();
    VectorValue::new(components).expect("bench MIPS query vector is valid")
}

fn recall_vector_components(seed: usize, dimension: usize) -> Vec<f32> {
    (0..dimension)
        .map(|dim| {
            if dim == 0 {
                seed as f32
            } else {
                let raw = seed
                    .wrapping_mul(dim.wrapping_mul(37).wrapping_add(11))
                    .wrapping_add(dim.wrapping_mul(31))
                    % 997;
                raw as f32 / 10_000.0
            }
        })
        .collect()
}

fn vector_components(seed: usize, dimension: usize) -> Vec<f32> {
    (0..dimension)
        .map(|dim| {
            let raw = (seed.wrapping_mul(31) + dim.wrapping_mul(17)) % 1_000;
            raw as f32 / 1_000.0
        })
        .collect()
}

fn overlap_count(exact: &[NodeId], approximate: &[VectorNodeSearchHit]) -> usize {
    exact
        .iter()
        .filter(|node_id| approximate.iter().any(|hit| hit.node_id == **node_id))
        .count()
}

fn recall_basis_points(recall: f64) -> u64 {
    (recall * 10_000.0).round() as u64
}

criterion_group! {
    name = graph_reads;
    config = common::criterion_config();
    targets = bench_node_fetch, bench_label_index, bench_typed_index_point,
        bench_typed_index_range, bench_composite_index_proxy, bench_exact_vector_scan,
        bench_hnsw_recall
}
criterion_main!(graph_reads);
