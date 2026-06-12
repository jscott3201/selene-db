#![allow(missing_docs)]
//! Criterion benches for TurboQuant query behavior after update/delete churn.

#[cfg(not(selene_bench_system_alloc))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use selene_core::{
    CancellationChecker, DbString, GraphId, LabelDiff, LabelSet, NodeId, PropertyDiff, PropertyMap,
    Value, VectorMetric, VectorValue, db_string,
};
use selene_graph::{
    ApproximateVectorSearchOptions, SharedGraph, VectorIndexConfig, VectorIndexKind,
};

const ROWS: usize = 10_000;
const DIMENSION: usize = 128;
const UPDATE_COUNT: usize = ROWS / 10;
const DELETE_COUNT: usize = ROWS / 20;
const K: usize = 10;
const SEARCH_WIDTH: usize = 512;

fn bench_turbo_quant_churn(c: &mut Criterion) {
    let fixture = TurboQuantChurnFixture::build();
    let mut group = c.benchmark_group("graph_turbo_quant_churn");
    group.bench_function(
        BenchmarkId::new(
            "tqcos_update10_delete5",
            format!("c{SEARCH_WIDTH}_n{}", compact_count(ROWS as u64)),
        ),
        |b| {
            b.iter(|| {
                std::hint::black_box(fixture.approximate_query_hit_count());
            });
        },
    );
    group.finish();
}

struct TurboQuantChurnFixture {
    shared: SharedGraph,
    label: DbString,
    embedding_key: DbString,
    query: VectorValue,
}

impl TurboQuantChurnFixture {
    fn build() -> Self {
        let label = db_string("TurboQuantChurnDoc").expect("bench label is valid");
        let embedding_key = db_string("embedding").expect("bench key is valid");
        let shared = SharedGraph::new(GraphId::new(70_000_000));
        let ids = seed_indexed_nodes(&shared, &label, &embedding_key);
        churn_indexed_nodes(&shared, &embedding_key, &ids);
        Self {
            shared,
            label,
            embedding_key,
            query: vector_value(ROWS - 1),
        }
    }

    fn approximate_query_hit_count(&self) -> usize {
        self.shared
            .approximate_vector_search_nodes_checked(
                &self.label,
                &self.embedding_key,
                &self.query,
                ApproximateVectorSearchOptions::new(VectorMetric::Cosine, K, SEARCH_WIDTH),
                CancellationChecker::disabled(),
            )
            .expect("bench TurboQuant query succeeds")
            .len()
    }
}

fn seed_indexed_nodes(
    shared: &SharedGraph,
    label: &DbString,
    embedding_key: &DbString,
) -> Vec<NodeId> {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    let mut ids = Vec::with_capacity(ROWS);
    for idx in 0..ROWS {
        let props =
            PropertyMap::from_pairs([(embedding_key.clone(), Value::Vector(vector_value(idx)))])
                .expect("bench vector properties are valid");
        ids.push(
            mutator
                .create_node(LabelSet::single(label.clone()), props)
                .expect("bench vector node insert succeeds"),
        );
    }
    mutator
        .create_vector_index_named_with_configs(
            label.clone(),
            embedding_key.clone(),
            VectorIndexKind::TurboQuantCosine,
            DIMENSION as u32,
            None,
            VectorIndexConfig::default(),
        )
        .expect("bench TurboQuant index build succeeds");
    txn.commit().expect("bench seed commit succeeds");
    ids
}

fn churn_indexed_nodes(shared: &SharedGraph, embedding_key: &DbString, ids: &[NodeId]) {
    let mut txn = shared.begin_write();
    let mut mutator = txn.mutator();
    for (offset, id) in ids.iter().copied().take(UPDATE_COUNT).enumerate() {
        mutator
            .update_node(
                id,
                LabelDiff::new([], []).expect("empty label diff is valid"),
                PropertyDiff::new(
                    [(
                        embedding_key.clone(),
                        Value::Vector(vector_value(ids.len() + offset)),
                    )],
                    [],
                )
                .expect("bench property diff is valid"),
            )
            .expect("bench vector update succeeds");
    }
    for id in ids.iter().copied().skip(UPDATE_COUNT).take(DELETE_COUNT) {
        mutator
            .delete_node(id)
            .expect("bench vector delete succeeds");
    }
    txn.commit().expect("bench churn commit succeeds");
}

fn compact_count(value: u64) -> String {
    if value >= 1_000 && value.is_multiple_of(1_000) {
        format!("{}k", value / 1_000)
    } else {
        value.to_string()
    }
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
    name = vector_turbo_churn;
    config = common::criterion_config();
    targets = bench_turbo_quant_churn
}
criterion_main!(vector_turbo_churn);
