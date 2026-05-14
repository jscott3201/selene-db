#![allow(missing_docs)]
//! Criterion benches for algorithms-pack CALL adapter overhead.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, intern};
use selene_gql::{
    ExecutionPlan, ProcedureRegistry, Session, StatementOutput, analyze, execute_statement, parse,
    plan,
};
use selene_graph::SharedGraph;
use selene_pack::ProcedurePackRegistry;
use selene_testing::BenchProfile;

const PROJECTION_BUILD: &str = "CALL algo.projection_build('p', NULL, NULL, NULL)";
const LARGE_GRAPH_SCALE: usize = 1_000;
const ALGORITHM_GRAPH_SCALE: usize = 256;
const APSP_GRAPH_SCALE: usize = 96;

fn bench_projection_build(c: &mut Criterion) {
    c.bench_function("algo_pack/projection_build_default", |b| {
        b.iter_batched(
            || BenchState::new(LARGE_GRAPH_SCALE, 76_001),
            |state| std::hint::black_box(state.execute(PROJECTION_BUILD)),
            BatchSize::SmallInput,
        );
    });
}

fn bench_pagerank(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_002);
    c.bench_function("algo_pack/algo_pagerank_default", |b| {
        b.iter(|| {
            std::hint::black_box(
                state.execute("CALL algo.pagerank('p', NULL, NULL, NULL) YIELD node_id, score"),
            );
        });
    });
}

fn bench_dijkstra(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_003);
    c.bench_function("algo_pack/algo_dijkstra_single_pair", |b| {
        b.iter(|| {
            std::hint::black_box(state.execute(
                "MATCH (source:Source), (target:Target) CALL algo.dijkstra('p', source, target) \
                 YIELD cost RETURN cost",
            ));
        });
    });
}

fn bench_apsp(c: &mut Criterion) {
    let state = BenchState::with_projection(APSP_GRAPH_SCALE, 76_004);
    c.bench_function("algo_pack/algo_apsp_default", |b| {
        b.iter(|| {
            std::hint::black_box(state.execute("CALL algo.apsp('p', 96) YIELD source_node, cost"));
        });
    });
}

fn bench_betweenness(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_005);
    c.bench_function("algo_pack/algo_betweenness_default", |b| {
        b.iter(|| {
            std::hint::black_box(
                state.execute("CALL algo.betweenness('p', NULL) YIELD node_id, score"),
            );
        });
    });
}

fn bench_louvain(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_006);
    c.bench_function("algo_pack/algo_louvain_default", |b| {
        b.iter(|| {
            std::hint::black_box(
                state.execute("CALL algo.louvain('p', NULL) YIELD node_id, community, level"),
            );
        });
    });
}

fn bench_triangle_count(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_007);
    c.bench_function("algo_pack/algo_triangle_count_default", |b| {
        b.iter(|| {
            std::hint::black_box(
                state.execute("CALL algo.triangle_count('p') YIELD node_id, triangle_count"),
            );
        });
    });
}

fn bench_label_propagation(c: &mut Criterion) {
    let state = BenchState::with_projection(ALGORITHM_GRAPH_SCALE, 76_008);
    c.bench_function("algo_pack/algo_label_propagation_default", |b| {
        b.iter(|| {
            std::hint::black_box(
                state.execute("CALL algo.label_propagation('p', NULL) YIELD node_id, community"),
            );
        });
    });
}

struct BenchState {
    graph: SharedGraph,
    registry: ProcedurePackRegistry,
}

impl BenchState {
    fn new(scale: usize, graph_id: u64) -> Self {
        let pack = AlgorithmsPack::new();
        let registry = pack
            .registry_with_builtins()
            .expect("algorithms pack registry builds");
        Self {
            graph: graph_fixture(scale, graph_id),
            registry,
        }
    }

    fn with_projection(scale: usize, graph_id: u64) -> Self {
        let state = Self::new(scale, graph_id);
        state.execute(PROJECTION_BUILD);
        state
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
}

fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("bench query parses");
    let analyzed = analyze(statement, registry, None).expect("bench query analyzes");
    plan(&analyzed, registry).expect("bench query plans")
}

fn graph_fixture(scale: usize, graph_id: u64) -> SharedGraph {
    let scale = scale.max(2);
    let graph = SharedGraph::new(GraphId::new(graph_id));
    let source_label = istr("Source");
    let target_label = istr("Target");
    let person_label = istr("Person");
    let rel = istr("LINK");
    let mut txn = graph.begin_write();
    let mut nodes = Vec::with_capacity(scale);
    for idx in 0..scale {
        let label = if idx == 0 {
            source_label
        } else if idx + 1 == scale {
            target_label
        } else {
            person_label
        };
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(label), PropertyMap::new())
                .expect("bench node inserts"),
        );
    }
    for idx in 0..scale {
        for target in deterministic_targets(idx, scale) {
            create_edge(&mut txn, rel, nodes[idx], nodes[target]);
        }
    }
    txn.commit().expect("bench graph commits");
    graph
}

fn deterministic_targets(source: usize, scale: usize) -> [usize; 3] {
    [
        (source + 1) % scale,
        (source + 7) % scale,
        (source + 31) % scale,
    ]
}

fn create_edge(txn: &mut selene_graph::WriteTxn<'_>, rel: IStr, source: NodeId, target: NodeId) {
    txn.mutator()
        .create_edge(rel, source, target, PropertyMap::new())
        .expect("bench edge inserts");
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
    targets = bench_projection_build,
        bench_pagerank,
        bench_dijkstra,
        bench_apsp,
        bench_betweenness,
        bench_louvain,
        bench_triangle_count,
        bench_label_propagation
}
criterion_main!(benches);

#[cfg(test)]
mod tests {
    #[test]
    fn benches_compile() {}
}
