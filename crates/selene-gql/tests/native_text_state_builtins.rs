//! End-to-end coverage for text scoring over maintained candidate state.

use std::sync::Arc;

use selene_core::{GraphId, IStr, LabelSet, NodeId, PropertyMap, Value, intern};
use selene_gql::{
    BindingTable, BuiltinProcedureRegistry, ExecutorError, ProcedureError, ProcedureRegistry,
    Session, StatementOutput,
};
use selene_graph::{
    CandidateStateSpec, IndexProvider, MaintainedCandidateStateProvider, SharedGraph,
};

fn istr(value: &str) -> IStr {
    intern(value).expect("test string interns")
}

fn graph(id: u64) -> SharedGraph {
    let doc = istr("TextDoc");
    let negative = istr("NEGATIVE");
    let provider = Arc::new(
        MaintainedCandidateStateProvider::new([CandidateStateSpec::new(istr("current_docs"))
            .require_label(doc)
            .exclude_outgoing(negative)])
        .expect("candidate-state provider is valid"),
    );
    SharedGraph::builder(GraphId::new(id))
        .with_provider(provider as Arc<dyn IndexProvider>)
        .build()
        .expect("graph builds")
}

fn props(key: &IStr, value: Value) -> PropertyMap {
    PropertyMap::from_pairs([(key.clone(), value)]).expect("test property map is valid")
}

fn node_list(nodes: &[NodeId]) -> Value {
    Value::List(nodes.iter().copied().map(Value::NodeRef).collect())
}

fn rows(output: StatementOutput) -> BindingTable {
    match output {
        StatementOutput::Rows(table) => table,
        StatementOutput::Written(outcome) => outcome.rows.expect("written statement returned rows"),
        other => panic!("expected rows, got {other:?}"),
    }
}

fn execute_rows(
    session: &mut Session<'_>,
    source: &str,
    registry: &dyn ProcedureRegistry,
) -> BindingTable {
    rows(
        session
            .execute_source(source, registry)
            .expect("statement executes"),
    )
}

fn execute_ok(session: &mut Session<'_>, source: &str, registry: &dyn ProcedureRegistry) {
    session
        .execute_source(source, registry)
        .expect("statement executes");
}

fn node_column(table: &BindingTable, name: &str) -> Vec<NodeId> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::NodeRef(value)) => *value,
            other => panic!("expected node ref in {name}, got {other:?}"),
        })
        .collect()
}

fn uint_column(table: &BindingTable, name: &str) -> Vec<u64> {
    let index = table
        .column_index(istr(name))
        .unwrap_or_else(|| panic!("missing column {name}"));
    table
        .rows()
        .iter()
        .map(|row| match row.values().get(index) {
            Some(Value::Uint(value)) => *value,
            other => panic!("expected uint in {name}, got {other:?}"),
        })
        .collect()
}

#[test]
fn text_score_candidate_state_expanded_batch_filters_stale_candidates() {
    let graph = graph(431_601);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    let (graph_root, memory_root, current_graph, current_memory) = seed_graph(&graph);
    execute_ok(
        &mut session,
        "CALL selene.create_text_index('TextDoc', 'body', 'body_idx')",
        &registry,
    );
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::String(istr("graph")),
            Value::String(istr("memory")),
        ]),
    );
    session.bind_parameter(
        istr("roots"),
        Value::List(vec![node_list(&[graph_root]), node_list(&[memory_root])]),
    );

    let table = execute_rows(
        &mut session,
        "CALL selene.text_score_candidate_state_expanded_batch( \
            'TextDoc', 'body', $queries, 'current_docs', $roots, 'SUPPORTS', 2) \
         YIELD query_index, node_id, score",
        &registry,
    );

    assert_eq!(uint_column(&table, "query_index"), vec![0, 1]);
    assert_eq!(
        node_column(&table, "node_id"),
        vec![current_graph, current_memory]
    );
}

#[test]
fn text_score_candidate_state_expanded_batch_rejects_mismatched_roots() {
    let graph = graph(431_602);
    let registry = BuiltinProcedureRegistry::new();
    let mut session = Session::new(&graph);
    session.bind_parameter(
        istr("queries"),
        Value::List(vec![
            Value::String(istr("graph")),
            Value::String(istr("memory")),
        ]),
    );
    session.bind_parameter(
        istr("roots"),
        Value::List(vec![node_list(&[NodeId::new(1)])]),
    );

    let err = session
        .execute_source(
            "CALL selene.text_score_candidate_state_expanded_batch( \
                'TextDoc', 'body', $queries, 'current_docs', $roots, 'SUPPORTS', 2)",
            &registry,
        )
        .expect_err("mismatched text-state batch lengths must fail");

    assert!(matches!(
        err,
        ExecutorError::Procedure {
            source: ProcedureError::InvalidArgument { ref detail },
            ..
        } if detail.contains("queries and roots must have the same length")
    ));
}

fn seed_graph(graph: &SharedGraph) -> (NodeId, NodeId, NodeId, NodeId) {
    let root = istr("TextRoot");
    let doc = istr("TextDoc");
    let body = istr("body");
    let supports = istr("SUPPORTS");
    let negative = istr("NEGATIVE");
    let mut txn = graph.begin_write();
    let mut mutator = txn.mutator();
    let graph_root = mutator
        .create_node(LabelSet::single(root.clone()), PropertyMap::new())
        .expect("graph root inserts");
    let memory_root = mutator
        .create_node(LabelSet::single(root), PropertyMap::new())
        .expect("memory root inserts");
    let current_graph = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&body, Value::String(istr("graph current fact"))),
        )
        .expect("current graph doc inserts");
    let stale_graph = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&body, Value::String(istr("graph graph stale fact"))),
        )
        .expect("stale graph doc inserts");
    let current_memory = mutator
        .create_node(
            LabelSet::single(doc.clone()),
            props(&body, Value::String(istr("memory current fact"))),
        )
        .expect("current memory doc inserts");
    let stale_memory = mutator
        .create_node(
            LabelSet::single(doc),
            props(&body, Value::String(istr("memory memory stale fact"))),
        )
        .expect("stale memory doc inserts");
    for target in [current_graph, stale_graph] {
        mutator
            .create_edge(supports.clone(), graph_root, target, PropertyMap::new())
            .expect("graph support edge inserts");
    }
    for target in [current_memory, stale_memory] {
        mutator
            .create_edge(supports.clone(), memory_root, target, PropertyMap::new())
            .expect("memory support edge inserts");
    }
    for stale in [stale_graph, stale_memory] {
        mutator
            .create_edge(negative.clone(), stale, current_graph, PropertyMap::new())
            .expect("negative evidence edge inserts");
    }
    txn.commit().expect("seed commits");
    (graph_root, memory_root, current_graph, current_memory)
}
