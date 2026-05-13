#![allow(dead_code)]

use selene_algorithms::{PageRankConfig, ProjectionCatalog, ProjectionConfig, pagerank};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, ExecutionPlan, ExecutorError, ProcedureRegistry, Session, StatementOutput,
    analyze, execute_statement, parse, plan,
};
use selene_graph::SharedGraph;
use selene_pack::ProcedurePackRegistry;

pub fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

pub fn registry(pack: &AlgorithmsPack) -> ProcedurePackRegistry {
    pack.registry_with_builtins()
        .expect("algorithms pack registers cleanly")
}

pub fn planned(source: &str, registry: &dyn ProcedureRegistry) -> ExecutionPlan {
    let statement = parse(source).expect("test input parses");
    let analyzed = analyze(statement, registry, None).expect("test input analyzes");
    plan(&analyzed, registry).expect("test input plans")
}

pub fn analyze_err(source: &str, registry: &dyn ProcedureRegistry) -> String {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry, None)
        .expect_err("test input fails analyze")
        .to_string()
}

pub fn execute_result(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> Result<StatementOutput, ExecutorError> {
    let plan = planned(source, registry);
    let mut session = Session::new(graph);
    execute_statement(&plan, &mut session, registry)
}

pub fn execute_ok(
    source: &str,
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
) -> StatementOutput {
    execute_result(source, graph, registry).expect("statement executes")
}

pub fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        other => panic!("expected rows, got {other:?}"),
    }
}

pub fn graph_with_edges(id: u64, edges: &[(usize, usize)]) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(id));
    let person = istr("Person");
    let rel = istr("LINK");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::new();
    for _ in 0..node_count(edges) {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(person), PropertyMap::new())
                .expect("fixture node inserts"),
        );
    }
    for &(source, target) in edges {
        txn.mutator()
            .create_edge(rel, nodes[source], nodes[target], PropertyMap::new())
            .expect("fixture edge inserts");
    }
    txn.commit().expect("fixture commit succeeds");
    (shared, nodes)
}

pub fn build_projection(graph: &SharedGraph, registry: &dyn ProcedureRegistry, name: &str) {
    execute_ok(
        &format!("CALL algo.projection_build('{name}', NULL, NULL, NULL)"),
        graph,
        registry,
    );
}

pub fn direct_pagerank_rows(graph: &SharedGraph, name: &str, config: PageRankConfig) -> Vec<Value> {
    let snapshot = graph.read();
    let catalog = ProjectionCatalog::new();
    catalog
        .project(
            &snapshot,
            &ProjectionConfig {
                name: name.to_string(),
                node_labels: Vec::new(),
                edge_labels: Vec::new(),
                weight_property: None,
            },
            None,
        )
        .expect("projection builds");
    let projection = catalog.get(name).expect("projection exists");
    pagerank(projection.projection(), config)
        .into_iter()
        .flat_map(|(node, score)| [Value::NodeRef(node), Value::Float(score)])
        .collect()
}

pub fn value_string(value: &Value) -> &str {
    match value {
        Value::String(value) => value.as_str(),
        Value::ExternalString(value) => value.as_ref(),
        other => panic!("expected string value, got {other:?}"),
    }
}

fn node_count(edges: &[(usize, usize)]) -> usize {
    edges
        .iter()
        .flat_map(|(source, target)| [*source, *target])
        .max()
        .map_or(1, |max| max + 1)
}
