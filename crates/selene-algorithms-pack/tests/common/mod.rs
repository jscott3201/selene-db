#![allow(dead_code)]

use selene_algorithms::{
    GraphProjection, PageRankConfig, ProjectionCatalog, ProjectionConfig, pagerank,
};
use selene_algorithms_pack::AlgorithmsPack;
use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    AnalysisError, BindingTable, ExecutionPlan, ExecutorError, ProcedureRegistry, Session,
    StatementOutput, analyze, execute_statement, parse, plan,
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
    analyze_failure(source, registry).to_string()
}

pub fn analyze_failure(source: &str, registry: &dyn ProcedureRegistry) -> AnalysisError {
    let statement = parse(source).expect("test input parses");
    analyze(statement, registry, None).expect_err("test input fails analyze")
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

pub fn graph_with_labeled_weighted_edges(
    id: u64,
    labels: &[&str],
    edges: &[(usize, usize, f64)],
) -> (SharedGraph, Vec<NodeId>) {
    graph_with_labeled_edges(id, labels, edges, true)
}

pub fn graph_with_labeled_unweighted_edges(
    id: u64,
    labels: &[&str],
    edges: &[(usize, usize)],
) -> (SharedGraph, Vec<NodeId>) {
    let weighted_edges: Vec<_> = edges
        .iter()
        .map(|&(source, target)| (source, target, 1.0))
        .collect();
    graph_with_labeled_edges(id, labels, &weighted_edges, false)
}

pub fn build_weighted_projection(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    name: &str,
    node_labels: &[&str],
) {
    build_projection_with_options(graph, registry, name, node_labels, Some("w"));
}

pub fn build_unweighted_projection(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    name: &str,
    node_labels: &[&str],
) {
    build_projection_with_options(graph, registry, name, node_labels, None);
}

pub fn build_empty_projection(graph: &SharedGraph, registry: &dyn ProcedureRegistry, name: &str) {
    build_projection_with_options(graph, registry, name, &["MissingLabel"], None);
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

pub fn direct_algorithm_rows(
    graph: &SharedGraph,
    name: &str,
    node_labels: &[&str],
    weight_property: Option<&str>,
    f: impl FnOnce(&GraphProjection) -> Vec<Vec<Value>>,
) -> Vec<Vec<Value>> {
    let snapshot = graph.read();
    let catalog = ProjectionCatalog::new();
    catalog
        .project(
            &snapshot,
            &ProjectionConfig {
                name: name.to_string(),
                node_labels: node_labels.iter().map(|label| istr(label)).collect(),
                edge_labels: Vec::new(),
                weight_property: weight_property.map(istr),
            },
            None,
        )
        .expect("projection builds");
    let projection = catalog.get(name).expect("projection exists");
    f(projection.projection())
}

pub fn table_values(table: &BindingTable) -> Vec<Vec<Value>> {
    table
        .rows()
        .iter()
        .map(|row| row.values().to_vec())
        .collect()
}

pub fn assert_invalid_argument(err: ExecutorError) {
    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: selene_gql::ProcedureError::InvalidArgument { .. },
            ..
        }
    ));
}

pub fn invalid_argument_detail(err: &ExecutorError) -> &str {
    let ExecutorError::Procedure {
        source: selene_gql::ProcedureError::InvalidArgument { detail },
        ..
    } = err
    else {
        panic!("expected InvalidArgument procedure error, got {err:?}");
    };
    detail
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

fn graph_with_labeled_edges(
    id: u64,
    labels: &[&str],
    edges: &[(usize, usize, f64)],
    weighted: bool,
) -> (SharedGraph, Vec<NodeId>) {
    let shared = SharedGraph::new(GraphId::new(id));
    let rel = istr("LINK");
    let mut txn = shared.begin_write();
    let mut nodes = Vec::with_capacity(labels.len());
    for label in labels {
        nodes.push(
            txn.mutator()
                .create_node(LabelSet::single(istr(label)), PropertyMap::new())
                .expect("fixture node inserts"),
        );
    }
    for &(source, target, weight) in edges {
        let properties = if weighted {
            PropertyMap::from_pairs([(istr("w"), Value::Float(weight))])
                .expect("weight property builds")
        } else {
            PropertyMap::new()
        };
        txn.mutator()
            .create_edge(rel, nodes[source], nodes[target], properties)
            .expect("fixture edge inserts");
    }
    txn.commit().expect("fixture commit succeeds");
    (shared, nodes)
}

fn build_projection_with_options(
    graph: &SharedGraph,
    registry: &dyn ProcedureRegistry,
    name: &str,
    node_labels: &[&str],
    weight_property: Option<&str>,
) {
    execute_ok(
        &format!(
            "CALL algo.projection_build('{name}', {}, NULL, {})",
            label_list(node_labels),
            nullable_string(weight_property)
        ),
        graph,
        registry,
    );
}

fn label_list(labels: &[&str]) -> String {
    if labels.is_empty() {
        return "NULL".to_string();
    }
    let rendered = labels
        .iter()
        .map(|label| format!("'{label}'"))
        .collect::<Vec<_>>();
    format!("[{}]", rendered.join(", "))
}

fn nullable_string(value: Option<&str>) -> String {
    value.map_or_else(|| "NULL".to_string(), |value| format!("'{value}'"))
}
