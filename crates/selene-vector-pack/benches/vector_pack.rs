#![allow(missing_docs)]
//! Criterion benches for vector-pack CALL adapter overhead.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use selene_core::{GraphId, IStr, LabelSet, PropertyMap, Value, intern};
use selene_gql::{
    ExecutionPlan, PipelineOp, ProcedureContext, ProcedureError, ProcedureHandle,
    ProcedureMutability, ProcedureRegistry, ProcedureResult, ProcedureTier, Session,
    StatementCategory, StatementOutput, analyze, execute_statement, parse, plan,
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

const SEARCH_DEFAULT: &str = "CALL vector.search('default', [1.0, 0.0, 0.0, 0.0], 10, NULL, NULL, NULL) YIELD node_id, score";
const IVF_SEARCH_DEFAULT: &str =
    "CALL vector.ivf_search('default', [0.0, 0.0], 10, NULL, NULL, NULL) YIELD node_id, score";
const IVF_STATS_DEFAULT: &str = "CALL vector.ivf_stats('default') YIELD state";

fn bench_search(c: &mut Criterion) {
    let state = BenchState::new_hnsw(HNSW_GRAPH_SCALE, 81_001);
    let plan = state.plan(SEARCH_DEFAULT);
    c.bench_function("vector_pack/search_default", |b| {
        b.iter(|| std::hint::black_box(state.execute_cached(&plan)));
    });
}

fn bench_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/upsert_default", |b| {
        b.iter_batched(
            || BenchRun::new_hnsw_mutation(1, 81_002, upsert_source(&hnsw_vectors(1, 81_102)[0])),
            |run| std::hint::black_box(run.execute()),
            BatchSize::SmallInput,
        );
    });
}

fn bench_bulk_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/bulk_upsert_default", |b| {
        b.iter_batched(
            || {
                let vectors = hnsw_vectors(BULK_BATCH_SIZE, 81_103);
                BenchRun::new_hnsw_mutation(BULK_BATCH_SIZE, 81_003, bulk_upsert_source(&vectors))
            },
            |run| std::hint::black_box(run.execute()),
            BatchSize::SmallInput,
        );
    });
}

fn bench_ivf_search(c: &mut Criterion) {
    let state = BenchState::new_ivf_trained(IVF_GRAPH_SCALE, 81_004);
    let plan = state.plan(IVF_SEARCH_DEFAULT);
    c.bench_function("vector_pack/ivf_search_default", |b| {
        b.iter(|| std::hint::black_box(state.execute_cached(&plan)));
    });
}

fn bench_ivf_bulk_upsert(c: &mut Criterion) {
    c.bench_function("vector_pack/ivf_bulk_upsert_default", |b| {
        b.iter_batched(
            || {
                let vectors = ivf_vectors(BULK_BATCH_SIZE, 81_105);
                BenchRun::new_ivf_mutation(
                    BULK_BATCH_SIZE,
                    81_005,
                    ivf_bulk_upsert_source(&vectors),
                )
            },
            |run| std::hint::black_box(run.execute()),
            BatchSize::SmallInput,
        );
    });
}

fn bench_ivf_stats(c: &mut Criterion) {
    let state = BenchState::new_ivf_trained(IVF_GRAPH_SCALE, 81_006);
    let plan = state.plan(IVF_STATS_DEFAULT);
    c.bench_function("vector_pack/ivf_stats_default", |b| {
        b.iter(|| std::hint::black_box(state.execute_cached(&plan)));
    });
}

struct BenchState {
    graph: SharedGraph,
    registry: ProcedurePackRegistry,
}

struct BenchRun {
    state: BenchState,
    plan: ExecutionPlan,
}

impl BenchRun {
    fn new_hnsw_mutation(scale: usize, graph_id: u64, source: String) -> Self {
        let state = BenchState::empty_hnsw(scale, graph_id);
        let plan = state.mutation_plan(&source);
        Self { state, plan }
    }

    fn new_ivf_mutation(scale: usize, graph_id: u64, source: String) -> Self {
        let state = BenchState::empty_ivf(scale, graph_id);
        let plan = state.mutation_plan(&source);
        Self { state, plan }
    }

    fn execute(&self) -> usize {
        self.state.execute_cached(&self.plan)
    }
}

impl BenchState {
    fn empty_hnsw(scale: usize, graph_id: u64) -> Self {
        let provider = Arc::new(HnswProvider::new(HnswConfig::new(HNSW_DIM).unwrap()).unwrap());
        Self::with_provider(scale, graph_id, provider)
    }

    fn new_hnsw(scale: usize, graph_id: u64) -> Self {
        let state = Self::empty_hnsw(scale, graph_id);
        let source = bulk_upsert_source(&hnsw_vectors(scale, graph_id));
        let plan = state.mutation_plan(&source);
        state.execute_cached(&plan);
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
        let source = ivf_bulk_upsert_source(&ivf_vectors(scale, graph_id));
        let plan = state.mutation_plan(&source);
        state.execute_cached(&plan);
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
        create_nodes(&graph, scale.max(1));
        Self { graph, registry }
    }

    fn plan(&self, source: &str) -> ExecutionPlan {
        planned(source, &self.registry)
    }

    fn mutation_plan(&self, source: &str) -> ExecutionPlan {
        // Mutation procedures take NodeRef inputs, but v1.0 GQL has no NodeRef
        // literal. Plan a MATCH-fed CALL as read-tier, then restore the real
        // mutation metadata so execution still uses the public CALL pipeline.
        let planning_registry = ReadMirrorRegistry {
            inner: &self.registry,
        };
        let mut plan = planned(source, &planning_registry);
        plan.category = StatementCategory::DataModifying;
        patch_call_metadata(&mut plan, &self.registry);
        plan
    }

    fn execute_cached(&self, plan: &ExecutionPlan) -> usize {
        let mut session = Session::new(&self.graph);
        match execute_statement(plan, &mut session, &self.registry).expect("bench query executes") {
            StatementOutput::Rows(table) => table.row_count(),
            StatementOutput::Empty => 0,
            StatementOutput::Written(outcome) => {
                outcome.rows.as_ref().map_or(0, |table| table.row_count())
            }
            _ => panic!("unexpected statement output for bench query"),
        }
    }
}

struct ReadMirrorRegistry<'a> {
    inner: &'a dyn ProcedureRegistry,
}

impl ProcedureRegistry for ReadMirrorRegistry<'_> {
    fn lookup(&self, name: &[IStr]) -> Option<selene_gql::ProcedureMetadata> {
        self.inner.lookup(name).map(|mut metadata| {
            metadata.mutability = ProcedureMutability::Read;
            metadata.tier = ProcedureTier::Graph;
            metadata
        })
    }

    fn execute(
        &self,
        _handle: ProcedureHandle,
        _args: &[Value],
        _ctx: &mut ProcedureContext<'_, '_>,
    ) -> Result<ProcedureResult, ProcedureError> {
        unreachable!("read-mirror registry is only used during benchmark planning")
    }
}

fn patch_call_metadata(plan: &mut ExecutionPlan, registry: &dyn ProcedureRegistry) {
    for op in &mut plan.pipeline {
        match op {
            PipelineOp::Call(call) => {
                let metadata = registry
                    .lookup(call.procedure.as_ref())
                    .expect("planned procedure still registered");
                call.handle = metadata.handle;
                call.tier = metadata.tier;
                call.mutability = metadata.mutability;
                call.output_schema = metadata.output_schema;
            }
            PipelineOp::Union { rhs, .. } | PipelineOp::Chain(rhs) => {
                patch_call_metadata(rhs, registry);
            }
            _ => {}
        }
    }
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("bench query parses");
    let analyzed = analyze(statement, registry, None).expect("bench query analyzes");
    plan(&analyzed, registry).expect("bench query plans")
}

fn create_nodes(graph: &SharedGraph, count: usize) {
    let mut txn = graph.begin_write();
    {
        let mut mutator = txn.mutator();
        for _ in 0..count {
            mutator
                .create_node(LabelSet::single(istr("Vec")), PropertyMap::new())
                .expect("bench node inserts");
        }
    }
    txn.commit().expect("bench graph commits");
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

fn upsert_source(vector: &[f64; HNSW_DIM]) -> String {
    format!(
        "MATCH (n:Vec) CALL vector.upsert('default', n, {})",
        vector_literal(vector)
    )
}

fn bulk_upsert_source(vectors: &[[f64; HNSW_DIM]]) -> String {
    format!(
        "MATCH (n:Vec) WITH collect(n) AS nodes CALL vector.bulk_upsert('default', nodes, {})",
        matrix_literal(vectors)
    )
}

fn ivf_bulk_upsert_source(vectors: &[[f64; IVF_DIM]]) -> String {
    format!(
        "MATCH (n:Vec) WITH collect(n) AS nodes CALL vector.ivf_bulk_upsert('default', nodes, {})",
        matrix_literal(vectors)
    )
}

fn vector_literal<const N: usize>(row: &[f64; N]) -> String {
    let values = row
        .iter()
        .map(|value| format!("{value:.8}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn matrix_literal<const N: usize>(rows: &[[f64; N]]) -> String {
    let rows = rows
        .iter()
        .map(vector_literal)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rows}]")
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
