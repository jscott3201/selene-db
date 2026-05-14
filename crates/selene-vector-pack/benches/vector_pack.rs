#![allow(missing_docs)]
//! Criterion benches for vector-pack CALL adapter overhead.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    ExecutionPlan, ImplDefinedCaps, MutationContext, ProcedureContext, ProcedureRegistry, Session,
    StatementOutput, analyze, execute_statement, parse, plan,
};
use selene_graph::{IndexProvider, SharedGraph, SubTag};
use selene_pack::ProcedurePackRegistry;
use selene_testing::BenchProfile;
use selene_vector::{
    DistanceMetric, HnswConfig, HnswProvider, IvfConfig, IvfProvider, IvfStats, PqParams,
};
use selene_vector_pack::VectorPack;

const HNSW_DIM: usize = 4;
const IVF_DIM: usize = 2;
const HNSW_GRAPH_SCALE: usize = 1_000;
const IVF_GRAPH_SCALE: usize = 256;
const BULK_BATCH_SIZE: usize = 100;

const SEARCH_DEFAULT: &str =
    "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 10, NULL, NULL) YIELD node_id, score";
const IVF_SEARCH_DEFAULT: &str =
    "CALL vector.ivf_search('default', [0.0, 0.0], 10, NULL, NULL) YIELD node_id, score";
const IVF_STATS_DEFAULT: &str = "CALL vector.ivf_stats('default') YIELD state";

fn bench_search(c: &mut Criterion) {
    let state = BenchState::new_hnsw(HNSW_GRAPH_SCALE, 81_001);
    c.bench_function("vector_pack/search_default", |b| {
        b.iter(|| std::hint::black_box(state.execute(SEARCH_DEFAULT)));
    });
}

fn bench_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/upsert_default", |b| {
        b.iter_batched(
            || BenchState::empty_hnsw(1, 81_002),
            |state| {
                std::hint::black_box(state.execute_mutation(
                    &["vector", "upsert"],
                    upsert_args(
                        state.node_ids[0],
                        hnsw_vector_value(&hnsw_vectors(1, 81_102)[0]),
                    ),
                ));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_bulk_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/bulk_upsert_default", |b| {
        b.iter_batched(
            || BenchState::empty_hnsw(BULK_BATCH_SIZE, 81_003),
            |state| {
                std::hint::black_box(state.execute_mutation(
                    &["vector", "bulk_upsert"],
                    bulk_upsert_args(&state.node_ids, hnsw_vectors(BULK_BATCH_SIZE, 81_103)),
                ));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_ivf_search(c: &mut Criterion) {
    let state = BenchState::new_ivf_trained(IVF_GRAPH_SCALE, 81_004);
    c.bench_function("vector_pack/ivf_search_default", |b| {
        b.iter(|| std::hint::black_box(state.execute(IVF_SEARCH_DEFAULT)));
    });
}

fn bench_ivf_bulk_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/ivf_bulk_upsert_default", |b| {
        b.iter_batched(
            || BenchState::empty_ivf(BULK_BATCH_SIZE, 81_005),
            |state| {
                std::hint::black_box(state.execute_mutation(
                    &["vector", "ivf_bulk_upsert"],
                    ivf_bulk_upsert_args(&state.node_ids, ivf_vectors(BULK_BATCH_SIZE, 81_105)),
                ));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_ivf_stats(c: &mut Criterion) {
    let state = BenchState::new_ivf_trained(IVF_GRAPH_SCALE, 81_006);
    c.bench_function("vector_pack/ivf_stats_default", |b| {
        b.iter(|| std::hint::black_box(state.execute(IVF_STATS_DEFAULT)));
    });
}

struct BenchState {
    graph: SharedGraph,
    registry: ProcedurePackRegistry,
    node_ids: Vec<NodeId>,
}

impl BenchState {
    fn empty_hnsw(scale: usize, graph_id: u64) -> Self {
        let provider = Arc::new(HnswProvider::new(HnswConfig::new(HNSW_DIM).unwrap()).unwrap());
        Self::with_provider(scale, graph_id, provider)
    }

    fn new_hnsw(scale: usize, graph_id: u64) -> Self {
        let state = Self::empty_hnsw(scale, graph_id);
        state.execute_mutation(
            &["vector", "bulk_upsert"],
            bulk_upsert_args(&state.node_ids, hnsw_vectors(scale, graph_id)),
        );
        state
    }

    fn empty_ivf(scale: usize, graph_id: u64) -> Self {
        let provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
        Self::with_provider(scale, graph_id, provider)
    }

    fn new_ivf_trained(scale: usize, graph_id: u64) -> Self {
        let scale = scale.max(IVF_GRAPH_SCALE);
        let provider = Arc::new(IvfProvider::new(ivf_config()).unwrap());
        let state = Self::with_provider(scale, graph_id, Arc::clone(&provider));
        state.execute_mutation(
            &["vector", "ivf_bulk_upsert"],
            ivf_bulk_upsert_args(&state.node_ids, ivf_vectors(scale, graph_id)),
        );
        train_ivf(&provider);
        debug_assert!(matches!(
            provider.ivf_stats().unwrap().unwrap(),
            IvfStats::Trained { .. }
        ));
        state
    }

    fn with_provider<P>(scale: usize, graph_id: u64, provider: Arc<P>) -> Self
    where
        P: IndexProvider + 'static,
    {
        let pack = VectorPack::new();
        let registry = pack
            .registry_with_builtins()
            .expect("vector pack registry builds");
        let graph = SharedGraph::builder(GraphId::new(graph_id))
            .with_provider(provider as Arc<dyn IndexProvider>)
            .build()
            .expect("bench graph builds");
        let node_ids = create_nodes(&graph, scale.max(1));
        Self {
            graph,
            registry,
            node_ids,
        }
    }

    fn execute(&self, source: &str) -> usize {
        let plan = planned(source, &self.registry);
        let mut session = Session::new(&self.graph);
        match execute_statement(&plan, &mut session, &self.registry).expect("bench query executes")
        {
            StatementOutput::Rows(table) => table.row_count(),
            StatementOutput::Empty => 0,
            _ => panic!("unexpected statement output for bench query"),
        }
    }

    fn execute_mutation(&self, name: &[&str], args: Vec<Value>) -> usize {
        let interned = name.iter().map(|segment| istr(segment)).collect::<Vec<_>>();
        let metadata = self
            .registry
            .lookup(&interned)
            .expect("mutation procedure registered");
        let mut txn = self.graph.begin_write();
        let caps = ImplDefinedCaps::default();
        let result = {
            let mut ctx =
                ProcedureContext::Mutation(MutationContext::for_test(txn.mutator(), &caps));
            self.registry.execute(metadata.handle, &args, &mut ctx)
        };
        match result {
            Ok(result) => {
                txn.commit().expect("mutation commit succeeds");
                result.rows.len()
            }
            Err(err) => {
                txn.rollback();
                panic!("bench mutation failed: {err:?}");
            }
        }
    }
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("bench query parses");
    let analyzed = analyze(statement, registry, None).expect("bench query analyzes");
    plan(&analyzed, registry).expect("bench query plans")
}

fn create_nodes(graph: &SharedGraph, count: usize) -> Vec<NodeId> {
    let mut txn = graph.begin_write();
    let mut node_ids = Vec::with_capacity(count);
    {
        let mut mutator = txn.mutator();
        for _ in 0..count {
            node_ids.push(
                mutator
                    .create_node(LabelSet::single(istr("Vec")), PropertyMap::new())
                    .expect("bench node inserts"),
            );
        }
    }
    txn.commit().expect("bench graph commits");
    node_ids
}

fn hnsw_vectors(count: usize, seed: u64) -> Vec<[f64; HNSW_DIM]> {
    let mut rng = fastrand::Rng::with_seed(seed);
    (0..count)
        .map(|_| {
            [
                f64::from(rng.f32()) + 0.001,
                f64::from(rng.f32()),
                f64::from(rng.f32()),
                f64::from(rng.f32()),
            ]
        })
        .collect()
}

fn ivf_vectors(count: usize, seed: u64) -> Vec<[f64; IVF_DIM]> {
    (0..count)
        .map(|idx| {
            let first = (idx as f64) + ((seed % 17) as f64 * 0.001);
            [first, (idx % 11) as f64]
        })
        .collect()
}

fn hnsw_vector_value(row: &[f64; HNSW_DIM]) -> Value {
    Value::List(row.iter().copied().map(Value::Float).collect())
}

fn hnsw_matrix(rows: Vec<[f64; HNSW_DIM]>) -> Value {
    Value::List(
        rows.iter()
            .map(|row| Value::List(row.iter().copied().map(Value::Float).collect()))
            .collect(),
    )
}

fn ivf_matrix(rows: Vec<[f64; IVF_DIM]>) -> Value {
    Value::List(
        rows.iter()
            .map(|row| Value::List(row.iter().copied().map(Value::Float).collect()))
            .collect(),
    )
}

fn node_ref_list(node_ids: &[NodeId]) -> Value {
    Value::List(node_ids.iter().copied().map(Value::NodeRef).collect())
}

fn upsert_args(node_id: NodeId, vector: Value) -> Vec<Value> {
    vec![
        Value::String(istr("default")),
        Value::NodeRef(node_id),
        vector,
    ]
}

fn bulk_upsert_args(node_ids: &[NodeId], vectors: Vec<[f64; HNSW_DIM]>) -> Vec<Value> {
    vec![
        Value::String(istr("default")),
        node_ref_list(node_ids),
        hnsw_matrix(vectors),
    ]
}

fn ivf_bulk_upsert_args(node_ids: &[NodeId], vectors: Vec<[f64; IVF_DIM]>) -> Vec<Value> {
    vec![
        Value::String(istr("default")),
        node_ref_list(node_ids),
        ivf_matrix(vectors),
    ]
}

fn ivf_config() -> IvfConfig {
    IvfConfig::with_params(
        IVF_DIM,
        4,
        2,
        DistanceMetric::L2,
        PqParams {
            m_subspaces: 1,
            k_centroids: 256,
            train_min_vectors: IVF_GRAPH_SCALE,
            use_opq: false,
            use_polysemous: false,
            hamming_threshold_ratio: 0.5,
        },
        IVF_GRAPH_SCALE,
    )
    .unwrap()
}

fn train_ivf(provider: &IvfProvider) {
    provider
        .write_section(SubTag(*b"CQNT"))
        .expect("CQNT write trains");
    provider
        .write_section(SubTag(*b"IPQB"))
        .expect("IPQB write publishes codebook");
    provider
        .write_section(SubTag(*b"POST"))
        .expect("POST write publishes postings");
}

fn istr(value: &str) -> IStr {
    intern(value).expect("bench string interns")
}

fn criterion_config() -> Criterion {
    let profile = BenchProfile::from_env();
    Criterion::default()
        .sample_size(profile.sample_size())
        .warm_up_time(std::time::Duration::from_millis(100))
        .measurement_time(std::time::Duration::from_millis(match profile {
            BenchProfile::Quick => 500,
            BenchProfile::Full | BenchProfile::Stress => 1_500,
            _ => 500,
        }))
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = bench_search,
        bench_upsert,
        bench_bulk_upsert,
        bench_ivf_search,
        bench_ivf_bulk_upsert,
        bench_ivf_stats
}
criterion_main!(benches);

#[cfg(test)]
mod tests {
    #[test]
    fn benches_compile() {}
}
